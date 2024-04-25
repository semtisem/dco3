use async_trait::async_trait;
use dco3_crypto::{ChunkedEncryption, DracoonCrypto, DracoonRSACrypto, Encrypter};
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tracing::error;

use crate::{
    constants::{
        CHUNK_SIZE, DRACOON_API_PREFIX, FILES_S3_COMPLETE, FILES_S3_URLS, POLLING_START_DELAY,
        PUBLIC_BASE, PUBLIC_SHARES_BASE, PUBLIC_UPLOAD_SHARES,
    },
    nodes::{
        upload::{calculate_s3_url_count, parse_upload_options, StreamUploadInternal},
        CloneableUploadProgressCallback, GeneratePresignedUrlsRequest, PresignedUrlList,
        S3FileUploadPart, S3UploadStatus, UploadOptions, UploadProgressCallback,
    },
    utils::FromResponse,
    DracoonClientError, Public,
};

use super::{
    CompleteS3ShareUploadRequest, CreateShareUploadChannelRequest,
    CreateShareUploadChannelResponse, FileName, PublicEndpoint, PublicUpload, PublicUploadShare,
    S3ShareUploadStatus, UserFileKey,
};

#[async_trait]
impl<S: Send + Sync, R: AsyncRead + Send + Sync + Unpin+ 'static> PublicUpload<R> for PublicEndpoint<S> {
    async fn upload<'r>(
        &'r self,
        access_key: impl Into<String> + Send + Sync,
        share: PublicUploadShare,
        upload_options: UploadOptions,
        reader: BufReader<R>,
        mut callback: Option<UploadProgressCallback>,
        chunk_size: Option<usize>,
    ) -> Result<FileName, DracoonClientError> {
        let use_s3_storage = self.get_system_info().await?.use_s3_storage;
        let is_encrypted = share.is_encrypted.unwrap_or(false);

        let upload_fn = match (use_s3_storage, is_encrypted) {
            (true, true) => PublicUploadInternal::upload_to_s3_encrypted,
            (true, false) => PublicUploadInternal::upload_to_s3_unencrypted, 
            _ => unimplemented!("NFS upload not implemented") 
        };

        upload_fn(
            self,
            access_key.into(),
            &share,
            upload_options,
            reader,
            callback,
            chunk_size,
        )
        .await
    }
}

impl<S> StreamUploadInternal<S> for PublicEndpoint<S> {}

#[async_trait]
impl<S: Send + Sync, R: AsyncRead + Send + Sync + Unpin + 'static> PublicUploadInternal<R, S>
    for PublicEndpoint<S>
{
    async fn create_upload_channel(
        &self,
        access_key: String,
        create_file_upload_req: CreateShareUploadChannelRequest,
    ) -> Result<CreateShareUploadChannelResponse, DracoonClientError> {
        let url_part = format!(
            "{DRACOON_API_PREFIX}/{PUBLIC_BASE}/{PUBLIC_SHARES_BASE}/{PUBLIC_UPLOAD_SHARES}/{}",
            access_key
        );

        let url = self.client().build_api_url(&url_part);

        let response = self
            .client()
            .http
            .post(url)
            .json(&create_file_upload_req)
            .send()
            .await?;

        CreateShareUploadChannelResponse::from_response(response).await
    }

    async fn create_s3_upload_urls(
        &self,
        access_key: String,
        upload_id: String,
        generate_urls_req: GeneratePresignedUrlsRequest,
    ) -> Result<PresignedUrlList, DracoonClientError> {
        let url_part = format!(
            "{DRACOON_API_PREFIX}/{PUBLIC_BASE}/{PUBLIC_SHARES_BASE}/{PUBLIC_UPLOAD_SHARES}/{}/{FILES_S3_URLS}",
            access_key
        );

        let url = self.client().build_api_url(&url_part);

        let response = self
            .client()
            .http
            .post(url)
            .json(&generate_urls_req)
            .send()
            .await?;

        PresignedUrlList::from_response(response).await
    }

    async fn upload_to_s3_unencrypted(
        &self,
        access_key: String,
        share: &PublicUploadShare,
        upload_options: UploadOptions,
        mut reader: BufReader<R>,
        callback: Option<UploadProgressCallback>,
        chunk_size: Option<usize>,
    ) -> Result<FileName, DracoonClientError> {
        // parse upload options
        let (
            classification,
            timestamp_creation,
            timestamp_modification,
            expiration,
            resolution_strategy,
            keep_share_links,
        ) = parse_upload_options(&upload_options);

        let fm = upload_options.file_meta.clone();

        let chunk_size = chunk_size.unwrap_or(CHUNK_SIZE);

        // create upload channel
        let file_upload_req = CreateShareUploadChannelRequest::builder(fm.0.clone())
            .with_size(fm.1.clone())
            .with_timestamp_creation(timestamp_creation)
            .with_timestamp_modification(timestamp_modification)
            .with_direct_s3_upload(true)
            .build();

        let upload_channel =
            <PublicEndpoint<S> as PublicUploadInternal<R, S>>::create_upload_channel(
                self,
                access_key.clone(),
                file_upload_req,
            )
            .await?;

        let mut s3_parts = Vec::new();

        let (count_urls, last_chunk_size) = calculate_s3_url_count(fm.1.clone(), chunk_size as u64);
        let mut url_part: u32 = 1;

        let cloneable_callback = callback.map(CloneableUploadProgressCallback::new);

        if count_urls > 1 {
            while url_part < count_urls {
                let mut buffer = vec![0; chunk_size];

                match reader.read_exact(&mut buffer).await {
                    Ok(0) => break,
                    Ok(n) => {
                        buffer.truncate(n);
                        let chunk = bytes::Bytes::from(buffer);

                        let stream: async_stream::__private::AsyncStream<
                            Result<bytes::Bytes, std::io::Error>,
                            _,
                        > = async_stream::stream! {
                            yield Ok(chunk);
                        };

                        let url_req = GeneratePresignedUrlsRequest::new(
                            n.try_into().expect("size not larger than 32 MB"),
                            url_part,
                            url_part,
                        );
                        let url = 
                        <PublicEndpoint<S> as PublicUploadInternal<R, S>>::
                            create_s3_upload_urls(self, access_key.clone(), upload_channel.upload_id.clone(), url_req)
                            .await?;
                        let url = url.urls.first().expect("Creating S3 url failed");

                        // truncation is safe because chunk_size is 32 MB
                        #[allow(clippy::cast_possible_truncation, clippy::cast_lossless)]
                        let curr_pos: u64 = ((url_part - 1) * (chunk_size as u32)) as u64;

                        let e_tag = self
                            .upload_stream_to_s3(
                                Box::pin(stream),
                                url,
                                fm.clone(),
                                chunk_size,
                                Some(curr_pos),
                                cloneable_callback.clone(),
                            )
                            .await?;

                        s3_parts.push(S3FileUploadPart::new(url_part, e_tag));
                        url_part += 1;
                    }
                    Err(err) => {
                        error!("Error reading file: {}", err);
                        return Err(DracoonClientError::IoError);
                    }
                }
            }
        }

        // upload last chunk
        let mut buffer = vec![
            0;
            last_chunk_size
                .try_into()
                .expect("size not larger than 32 MB")
        ];
        match reader.read_exact(&mut buffer).await {
            Ok(n) => {
                buffer.truncate(n);
                let chunk = bytes::Bytes::from(buffer);
                let stream: async_stream::__private::AsyncStream<
                    Result<bytes::Bytes, std::io::Error>,
                    _,
                > = async_stream::stream! {
                    // TODO: chunk stream for better progress
                    // currently the progress is only updated per chunk
                    yield Ok(chunk);

                };

                let url_req = GeneratePresignedUrlsRequest::new(
                    n.try_into().expect("size not larger than 32 MB"),
                    url_part,
                    url_part,
                );
                let url = 
                <PublicEndpoint<S> as PublicUploadInternal<R, S>>::
                    create_s3_upload_urls(self, access_key.clone(), upload_channel.upload_id.clone(), url_req)
                    .await?;

                let url = url.urls.first().expect("Creating S3 url failed");

                let curr_pos: u64 = (url_part - 1) as u64 * (CHUNK_SIZE as u64);

                let e_tag = self
                    .upload_stream_to_s3(
                        Box::pin(stream),
                        url,
                        upload_options.file_meta.clone(),
                        n,
                        Some(curr_pos),
                        cloneable_callback.clone(),
                    )
                    .await?;

                s3_parts.push(S3FileUploadPart::new(url_part, e_tag));
            }
            Err(err) => {
                error!("Error reading file: {}", err);
                return Err(DracoonClientError::IoError);
            }
        }

        // finalize upload
        let complete_upload_req = CompleteS3ShareUploadRequest::new(s3_parts, None);

        <PublicEndpoint<S> as PublicUploadInternal<R, S>>::finalize_upload(self, access_key.clone(), upload_channel.upload_id.clone(), complete_upload_req)
            .await?;

        // get upload status
        // return node if upload is done
        // return error if upload failed
        // polling with exponential backoff
        let mut sleep_duration = POLLING_START_DELAY;
        loop {
            let status_response =  <PublicEndpoint<S> as PublicUploadInternal<R, S>>::
                get_upload_status(self, access_key.clone(), upload_channel.upload_id.clone())
                .await?;

            match status_response.status {
                S3UploadStatus::Done => {
                    return Ok(status_response.file_name);
                }
                S3UploadStatus::Error => {
                    let response = status_response
                        .error_details
                        .expect("Error message must be set if status is error");
                    error!("Error uploading file: {}", response);
                    return Err(DracoonClientError::Http(response));
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(sleep_duration)).await;
                    sleep_duration *= 2;
                }
            }
        }
    }
    async fn upload_to_s3_encrypted(
        &self,
        access_key: String,
        share: &PublicUploadShare,
        upload_options: UploadOptions,
        mut reader: BufReader<R>,
        mut callback: Option<UploadProgressCallback>,
        chunk_size: Option<usize>,
    ) -> Result<FileName, DracoonClientError> {

        let chunk_size = chunk_size.unwrap_or(CHUNK_SIZE);

        let mut crypto_buff =
            vec![0u8; upload_options.file_meta.1.try_into().expect("size not larger than 32 MB")];
        let mut read_buff = vec![0u8; upload_options.file_meta.1.try_into().expect("size not larger than 32 MB")];
        let mut crypter = DracoonCrypto::encrypter(&mut crypto_buff)?;

        while let Ok(chunk) = reader.read(&mut read_buff).await {
            if chunk == 0 {
                break;
            }
            crypter.update(&read_buff[..chunk])?;
        }
        crypter.finalize()?;
        // drop the read buffer after completing the encryption
        drop(read_buff);

        let enc_bytes = crypter.get_message().clone();

        assert_eq!(enc_bytes.len() as u64, upload_options.file_meta.1);

        let mut crypto_reader = BufReader::new(enc_bytes.as_slice());
        let plain_file_key = crypter.get_plain_file_key();

        // drop the crypto buffer (enc bytes are still in the reader)
        drop(crypto_buff);

        let public_keys = share.user_user_public_key_list.clone().unwrap_or_default();

        let user_file_keys: Vec<_> = public_keys.items.iter().flat_map(|key| {
            DracoonCrypto::encrypt_file_key(plain_file_key.clone(), key.public_key_container.clone())
                .map(|file_key| UserFileKey::new(key.id, file_key))
                .into_iter()  
        }).collect();

        let (
            classification,
            timestamp_creation,
            timestamp_modification,
            expiration,
            resolution_strategy,
            keep_share_links,
        ) = parse_upload_options(&upload_options);

        let fm = upload_options.file_meta.clone();

        // create upload channel
        let file_upload_req = CreateShareUploadChannelRequest::builder(fm.0.clone())
            .with_size(fm.1.clone())
            .with_timestamp_modification(timestamp_modification)
            .with_timestamp_creation(timestamp_creation)
            .build();

        let upload_channel = 
        <PublicEndpoint<S> as PublicUploadInternal<R, S>>::create_upload_channel
        (self, access_key.clone(), file_upload_req)
        .await
        .map_err(|err| {
            error!("Error creating upload channel: {}", err);
            err
        })?;

        let mut s3_parts = Vec::new();

        let (count_urls, last_chunk_size) = calculate_s3_url_count(fm.1, chunk_size as u64);
        let mut url_part: u32 = 1;

        let cloneable_callback = callback.map(CloneableUploadProgressCallback::new);

        if count_urls > 1 {
            while url_part < count_urls {
                let mut buffer = vec![0; chunk_size];

                match crypto_reader.read_exact(&mut buffer).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk_len = n;
                        buffer.truncate(chunk_len);
                        let chunk = bytes::Bytes::from(buffer);

                        let stream: async_stream::__private::AsyncStream<
                            Result<bytes::Bytes, std::io::Error>,
                            _,
                        > = async_stream::stream! {
                            yield Ok(chunk);
                        };

                        let url_req = GeneratePresignedUrlsRequest::new(
                            chunk_len.try_into().expect("size not larger than 32 MB"),
                            url_part,
                            url_part,
                        );
                        let url =
                             <PublicEndpoint<S> as PublicUploadInternal<R, S>>::create_s3_upload_urls::<
                                '_,
                                '_,
                            >(
                                self, access_key.clone(), upload_channel.upload_id.clone(), url_req
                            )
                            .await
                            .map_err(|err| {
                                error!("Error creating S3 upload urls: {}", err);
                                err
                            })?;
                        let url = url.urls.first().expect("Creating S3 url failed");

                        let curr_pos: u64 = (url_part - 1) as u64 * (chunk_size as u64);

                        let e_tag =  self.upload_stream_to_s3(
                            Box::pin(stream),
                            url,
                            upload_options.file_meta.clone(),
                            chunk_len,
                            Some(curr_pos),
                            cloneable_callback.clone(),
                        )
                        .await
                        .map_err(|err| {
                            error!("Error uploading stream to S3: {}", err);
                            err
                        })?;

                        s3_parts.push(S3FileUploadPart::new(url_part, e_tag));
                        url_part += 1;
                    }
                    Err(err) => return Err(DracoonClientError::IoError),
                }
            }
        }

        // upload last chunk
        let mut buffer = vec![
            0;
            last_chunk_size
                .try_into()
                .expect("size not larger than 32 MB")
        ];
        match crypto_reader.read_exact(&mut buffer).await {
            Ok(n) => {
                buffer.truncate(n);
                let chunk = bytes::Bytes::from(buffer);
                let stream: async_stream::__private::AsyncStream<
                    Result<bytes::Bytes, std::io::Error>,
                    _,
                > = async_stream::stream! {
                    // TODO: chunk stream for better progress
                    yield Ok(chunk);

                };

                let url_req = GeneratePresignedUrlsRequest::new(
                    n.try_into().expect("size not larger than 32 MB"),
                    url_part,
                    url_part,
                );
                let url =
                     <PublicEndpoint<S> as PublicUploadInternal<R, S>>::create_s3_upload_urls::<'_, '_>(
                        self,
                        access_key.clone(),
                        upload_channel.upload_id.clone(),
                        url_req,
                    )
                    .await
                    .map_err(|err| {
                        error!("Error creating S3 upload urls: {}", err);
                        err
                    })?;

                let url = url.urls.first().expect("Creating S3 url failed");

                // truncation is safe because chunk_size is 32 MB
                #[allow(clippy::cast_possible_truncation, clippy::cast_lossless)]
                let curr_pos: u64 = ((url_part - 1) * (CHUNK_SIZE as u32)) as u64;

                let e_tag =  self.upload_stream_to_s3(
                    Box::pin(stream),
                    url,
                    upload_options.file_meta.clone(),
                    n,
                    Some(curr_pos),
                    cloneable_callback.clone(),
                )
                .await
                .map_err(|err| {
                    error!("Error uploading stream to S3: {}", err);
                    err
                })?;

                s3_parts.push(S3FileUploadPart::new(url_part, e_tag));
            }

            Err(err) => {
                error!("Error reading file: {}", err);
                return Err(DracoonClientError::IoError);
            }
        }

        // finalize upload
        let complete_upload_req = CompleteS3ShareUploadRequest::new(s3_parts, Some(user_file_keys));

         <PublicEndpoint<S> as PublicUploadInternal<R, S>>::finalize_upload::<'_, '_>(
            self,
            access_key.clone(),
            upload_channel.upload_id.clone(),
            complete_upload_req,
        )
        .await
        .map_err(|err| {
            error!("Error finalizing upload: {}", err);
            err
        })?;

        // get upload status
        // return node if upload is done
        // return error if upload failed
        // polling with exponential backoff
        let mut sleep_duration = POLLING_START_DELAY;
        loop {
            let status_response =  <PublicEndpoint<S> as PublicUploadInternal<R, S>>::get_upload_status(
                self,
                access_key.clone(),
                upload_channel.upload_id.clone(),
            )
            .await
            .map_err(|err| {
                error!("Error getting upload status: {}", err);
                err
            })?;

            match status_response.status {
                S3UploadStatus::Done => {
                    return Ok(status_response.file_name);

                }
                S3UploadStatus::Error => {
                    return Err(DracoonClientError::Http(
                        status_response
                            .error_details
                            .expect("Error message must be set if status is error"),
                    ));
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(sleep_duration)).await;
                    sleep_duration *= 2;
                }
            }
        }
    }

    async fn finalize_upload(
        &self,
        access_key: String,
        upload_id: String,
        complete_file_upload_req: CompleteS3ShareUploadRequest,
    ) -> Result<(), DracoonClientError> {
        let url_part = format!(
            "{DRACOON_API_PREFIX}/{PUBLIC_BASE}/{PUBLIC_SHARES_BASE}/{PUBLIC_UPLOAD_SHARES}/{}/{FILES_S3_COMPLETE}",
            access_key
        );

        let url = self.client().build_api_url(&url_part);

        let response = self
            .client()
            .http
            .put(url)
            .json(&complete_file_upload_req)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(DracoonClientError::from_response(response).await?)
        }
    }

    async fn get_upload_status(
        &self,
        access_key: String,
        upload_id: String,
    ) -> Result<S3ShareUploadStatus, DracoonClientError> {
        todo!()
    }
}

#[async_trait]
trait PublicUploadInternal<R: AsyncRead, S>: StreamUploadInternal<S> {
    async fn create_upload_channel(
        &self,
        access_key: String,
        create_file_upload_req: CreateShareUploadChannelRequest,
    ) -> Result<CreateShareUploadChannelResponse, DracoonClientError>;

    async fn create_s3_upload_urls(
        &self,
        access_key: String,
        upload_id: String,
        generate_urls_req: GeneratePresignedUrlsRequest,
    ) -> Result<PresignedUrlList, DracoonClientError>;

    async fn upload_to_s3_unencrypted(
        &self,
        access_key: String,
        share: &PublicUploadShare,
        upload_options: UploadOptions,
        reader: BufReader<R>,
        mut callback: Option<UploadProgressCallback>,
        chunk_size: Option<usize>,
    ) -> Result<FileName, DracoonClientError>;
    async fn upload_to_s3_encrypted(
        &self,
        access_key: String,
        share: &PublicUploadShare,
        upload_options: UploadOptions,
        reader: BufReader<R>,
        mut callback: Option<UploadProgressCallback>,
        chunk_size: Option<usize>,
    ) -> Result<FileName, DracoonClientError>;

    async fn finalize_upload(
        &self,
        access_key: String,
        upload_id: String,
        complete_file_upload_req: CompleteS3ShareUploadRequest,
    ) -> Result<(), DracoonClientError>;

    async fn get_upload_status(
        &self,
        access_key: String,
        upload_id: String,
    ) -> Result<S3ShareUploadStatus, DracoonClientError>;
}

#[async_trait]
trait PublicUploadInternalNfs<S>: StreamUploadInternal<S> {}
