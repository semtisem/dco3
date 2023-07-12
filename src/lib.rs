#![allow(dead_code)]
#![allow(unused_variables)]

//! # dco3 - DRACOON API wrapper in Rust
//!
//! `dco3` is an async wrapper around API calls in DRACOON.
//! DRACOON is a cloud service provider - more information can be found on <https://dracoon.com>.
//! The name is based on several other projects pointing to oxide (Rust) and DRACOON.
//!
//! ## Usage
//! All API calls are implemented by the `Dracoon` struct. It can be created by using the `builder()` method.
//! 
//! In order to access specific API calls, the `Dracoon` struct needs to be in the `Connected` state. 
//! This can be achieved by calling the `connect` method.
//! To use specific endpoints, you need to import relevant traits.
//! Currently, the following traits are implemented:
//! 
//! * [User] - for user account management
//! * [UserAccountKeypairs] - for user keypair management
//! * [Nodes] - for node operations (folders, rooms, upload and download are excluded)
//! * [Download] - for downloading files
//! * [Upload] - for uploading files
//! * [Folders] - for folder operations
//! * [Rooms] - for room operations
//! * [DownloadShares] - for download share operations
//! * [UploadShares] - for upload share operations
//! * [Groups] - for group operations
//! * [Users] - for user management operations
//! 
//! 
//! ### Example
//! ```no_run
//! use dco3::{Dracoon, OAuth2Flow, User};
//! 
//! #[tokio::main]
//! async fn main() {
//!    let dracoon = Dracoon::builder()
//!       .with_base_url("https://dracoon.team")
//!       .with_client_id("client_id")
//!       .with_client_secret("client_secret")
//!       .build()
//!       .unwrap()
//!       .connect(OAuth2Flow::password_flow("username", "password"))
//!       .await
//!       .unwrap();
//! 
//!   let user_info = dracoon.get_user_account().await.unwrap();
//!   println!("User info: {:?}", user_info);
//! }
//!```
//! 
//! ## Authentication
//! 
//! All supported OAuth2 flows are implemented. 
//! 
//! ### Password Flow
//! ```no_run
//! use dco3::{Dracoon, OAuth2Flow};
//! 
//! #[tokio::main]
//! async fn main() {
//! 
//!    // you can instantiate the required flow by using the `OAuth2Flow` enum
//!    let password_flow = OAuth2Flow::password_flow("username", "password");
//! 
//!    let dracoon = Dracoon::builder()
//!       .with_base_url("https://dracoon.team")
//!       .with_client_id("client_id")
//!       .with_client_secret("client_secret")
//!       .build()
//!       .unwrap()
//!       .connect(password_flow)
//!       .await
//!       .unwrap();
//! }
//!```
//! ### Authorization Code Flow
//! ```no_run
//! use dco3::{Dracoon, OAuth2Flow};
//! 
//! #[tokio::main]
//! async fn main() {
//! 
//!    let mut dracoon = Dracoon::builder()
//!       .with_base_url("https://dracoon.team")
//!       .with_client_id("client_id")
//!       .with_client_secret("client_secret")
//!       .with_redirect_uri("https://redirect.uri")
//!       .build()
//!       .unwrap();
//! 
//!    // initiate the authorization code flow
//!    let authorize_url = dracoon.get_authorize_url();
//! 
//!    // get auth code
//!    let auth_code = "some_auth_code";
//! 
//!    // you can instantiate the required flow by using the `OAuth2Flow` enum
//!    let auth_code_flow = OAuth2Flow::authorization_code(auth_code);
//! 
//!    let dracoon = dracoon.connect(auth_code_flow).await.unwrap();
//! }
//!```
//! 
//! ### Refresh Token
//! 
//! ```no_run
//! use dco3::{Dracoon, OAuth2Flow};
//! 
//! #[tokio::main]
//! async fn main() {
//! 
//!   let refresh_token = "some_refresh_token";
//! 
//!   let dracoon = Dracoon::builder()
//!     .with_base_url("https://dracoon.team")
//!     .with_client_id("client_id")
//!     .with_client_secret("client_secret")
//!     .build()
//!     .unwrap()
//!     .connect(OAuth2Flow::refresh_token(refresh_token))
//!     .await
//!     .unwrap();
//! 
//! }
//! ```
//! 
//! ## Error handling
//! 
//! All errors are wrapped in the [DracoonClientError] enum.
//! 
//! Most errrors are related to general usage (like missing parameters). 
//! 
//! All API errors are wrapped in the `DracoonClientError::Http` variant.
//! The variant contains response with relevant status code and message.
//! 
//! You can check if the underlying error message if a specific API error by using the `is_*` methods.
//! 
//! ```no_run
//! use dco3::{Dracoon, OAuth2Flow, Nodes};
//! 
//! #[tokio::main]
//! 
//! async fn main() {
//! 
//!  let dracoon = Dracoon::builder()
//!    .with_base_url("https://dracoon.team")
//!    .with_client_id("client_id")
//!    .with_client_secret("client_secret")
//!    .build()
//!    .unwrap()
//!    .connect(OAuth2Flow::PasswordFlow("username".into(), "password".into()))
//!    .await
//!    .unwrap();
//! 
//! let node = dracoon.get_node(123).await;
//! 
//! match node {
//!  Ok(node) => println!("Node info: {:?}", node),
//! Err(err) => {
//!  if err.is_not_found() {
//!     println!("Node not found");
//!     } else {
//!          println!("Error: {:?}", err);
//!            }
//!         }
//!       }
//!  }
//! 
//! ```
//! 
//! ### Retries 
//! The client will automatically retry failed requests.
//! You can configure the retry behavior by passing your config during client creation.
//! 
//! Default values are: 5 retries, min delay 600ms, max delay 20s.
//! Keep in mind that you cannot set arbitrary values - for all values, minimum and maximum values are defined.
//! 
//! ```
//! 
//! use dco3::{Dracoon, OAuth2Flow};
//! 
//! #[tokio::main]
//! async fn main() {
//! 
//!  let dracoon = Dracoon::builder()
//!   .with_base_url("https://dracoon.team")
//!   .with_client_id("client_id")
//!   .with_client_secret("client_secret")
//!   .with_max_retries(3)
//!   .with_min_retry_delay(400)
//!   .with_max_retry_delay(1000)
//!   .build();
//! 
//! }
//! 
//! ```
//! 
//! ## Building requests
//! 
//! All API calls are implemented as traits.
//! Each API call that requires a sepcific payload has a corresponding builder.
//! To access the builder, you can call the builder() method.
//! 
//! ```no_run
//! # use dco3::{Dracoon, OAuth2Flow, Rooms, nodes::CreateRoomRequest};
//! # #[tokio::main]
//! # async fn main() {
//! # let dracoon = Dracoon::builder()
//! #  .with_base_url("https://dracoon.team")
//! #  .with_client_id("client_id")
//! #  .with_client_secret("client_secret")
//! #  .build()
//! #  .unwrap()
//! #  .connect(OAuth2Flow::PasswordFlow("username".into(), "password".into()))
//! #  .await
//! #  .unwrap();
//! let room = CreateRoomRequest::builder("My Room")
//!            .with_parent_id(123)
//!            .with_admin_ids(vec![1, 2, 3])
//!            .build();
//! 
//! let room = dracoon.create_room(room).await.unwrap();
//! 
//! # }
//! ```
//! Some requests do not have any complicated fields - in these cases, use the `new()` method.
//! ```no_run
//! # use dco3::{Dracoon, OAuth2Flow, Groups, groups::CreateGroupRequest};
//! # #[tokio::main]
//! # async fn main() {
//! # let dracoon = Dracoon::builder()
//! #  .with_base_url("https://dracoon.team")
//! #  .with_client_id("client_id")
//! #  .with_client_secret("client_secret")
//! #  .build()
//! #  .unwrap()
//! #  .connect(OAuth2Flow::PasswordFlow("username".into(), "password".into()))
//! #  .await
//! #  .unwrap();
//! 
//! // this takes a mandatory name and optional expiration
//! let group = CreateGroupRequest::new("My Group", None);
//! let group = dracoon.create_group(group).await.unwrap();
//! 
//! # }
//! ```
//! 
//! ## Pagination
//! 
//! GET endpoints are limited to 500 returned items - therefore you must paginate the content to fetch 
//! remaining items.
//! 
//! ```no_run
//! # use dco3::{Dracoon, auth::OAuth2Flow, Nodes, ListAllParams};
//! # #[tokio::main]
//! # async fn main() {
//! # let dracoon = Dracoon::builder()
//! #  .with_base_url("https://dracoon.team")
//! #  .with_client_id("client_id")
//! #  .with_client_secret("client_secret")
//! #  .build()
//! #  .unwrap()
//! #  .connect(OAuth2Flow::PasswordFlow("username".into(), "password".into()))
//! #  .await
//! #  .unwrap(); 
//! 

//! // This fetches the first 500 nodes without any param
//!  let mut nodes = dracoon.get_nodes(None, None, None).await.unwrap();
//! 
//! // Iterate over the remaining nodes
//!  for offset in (0..nodes.range.total).step_by(500) {
//!  let params = ListAllParams::builder()
//!   .with_offset(offset)
//!   .build();
//!  let next_nodes = dracoon.get_nodes(None, None, Some(params)).await.unwrap();
//!  
//!   nodes.items.extend(next_nodes.items);
//! 
//! };
//! # }
//! ```
//! ## Cryptography support
//! All API calls (specifically up- and downloads) support encryption and decryption.
//! In order to use encryption, you need to pass the encryption password while building the client.
//! 
//! ```no_run
//!  use dco3::{Dracoon, OAuth2Flow};
//!  #[tokio::main]
//!  async fn main() {
//!  let dracoon = Dracoon::builder()
//!   .with_base_url("https://dracoon.team")
//!   .with_client_id("client_id")
//!   .with_client_secret("client_secret")
//!    .with_encryption_password("my secret")
//!   .build()
//!   .unwrap()
//!   .connect(OAuth2Flow::PasswordFlow("username".into(), "password".into()))
//!   .await
//!   .unwrap();
//! // check if the keypair is present (fails with error if no keypair is present)
//! let kp = dracoon.get_keypair().await.unwrap();
//! # }
//! ```
//! ## Examples
//! For an example client implementation, see the [dccmd-rs](https://github.com/unbekanntes-pferd/dccmd-rs) repository.

use std::marker::PhantomData;

use dco3_crypto::PlainUserKeyPairContainer;
use reqwest::Url;

use self::{
    auth::{Connected, Disconnected},
    auth::{DracoonClient, DracoonClientBuilder},
    user::models::UserAccount,
};

// re-export traits and base models
pub use self::{
    nodes::{Download, Folders, Nodes, Rooms, Upload},
    user::{User, UserAccountKeypairs},
    auth::errors::DracoonClientError,
    auth::OAuth2Flow,
    groups::Groups,
    shares::{DownloadShares, UploadShares},
    users::Users,
    models::*,
};


pub mod auth;
pub mod constants;
pub mod models;
pub mod nodes;
pub mod user;
pub mod utils;
pub mod groups;
pub mod shares;
pub mod users;


/// DRACOON struct - implements all API calls via traits
#[derive(Clone)]
pub struct Dracoon<State = Disconnected> {
    client: DracoonClient<State>,
    state: PhantomData<State>,
    user_info: Option<UserAccount>,
    keypair: Option<PlainUserKeyPairContainer>,
    encryption_secret: Option<String>,
}

/// Builder for the `Dracoon` struct.
/// Requires a base url, client id and client secret.
/// Optionally, a redirect uri can be provided.
/// For convenience, use the [Dracoon] builder method.
#[derive(Default)]
pub struct DracoonBuilder {
    client_builder: DracoonClientBuilder,
    encryption_secret: Option<String>,
}

impl DracoonBuilder {
    /// Creates a new `DracoonBuilder`
    pub fn new() -> Self {
        let client_builder = DracoonClientBuilder::new();
        Self {
            client_builder,
            encryption_secret: None,
        }
    }

    pub fn with_encryption_password(mut self, encryption_secret: impl Into<String>) -> Self {
        self.encryption_secret = Some(encryption_secret.into());
        self
    }

    /// Sets the base url for the DRACOON instance
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.client_builder = self.client_builder.with_base_url(base_url);
        self
    }

    /// Sets the client id for the DRACOON instance
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_builder = self.client_builder.with_client_id(client_id);
        self
    }

    /// Sets the client secret for the DRACOON instance
    pub fn with_client_secret(mut self, client_secret: impl Into<String>) -> Self {
        self.client_builder = self.client_builder.with_client_secret(client_secret);
        self
    }

    /// Sets the redirect uri for the DRACOON instance
    pub fn with_redirect_uri(mut self, redirect_uri: impl Into<String>) -> Self {
        self.client_builder = self.client_builder.with_redirect_uri(redirect_uri);
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.client_builder = self.client_builder.with_user_agent(user_agent);
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.client_builder = self.client_builder.with_max_retries(max_retries);
        self
    }

    pub fn with_min_retry_delay(mut self, min_retry_delay: u64) -> Self {
        self.client_builder = self.client_builder.with_min_retry_delay(min_retry_delay);
        self
    }

    pub fn with_max_retry_delay(mut self, max_retry_delay: u64) -> Self {
        self.client_builder = self.client_builder.with_max_retry_delay(max_retry_delay);
        self
    }

    /// Builds the `Dracoon` struct - fails, if any of the required fields are missing
    pub fn build(self) -> Result<Dracoon<Disconnected>, DracoonClientError> {
        let dracoon = self.client_builder.build()?;

        Ok(Dracoon {
            client: dracoon,
            state: PhantomData,
            user_info: None,
            keypair: None,
            encryption_secret: self.encryption_secret,
        })
    }
}

impl Dracoon<Disconnected> {

    pub fn builder() -> DracoonBuilder {
        DracoonBuilder::new()
    }

    pub async fn connect(
        self,
        oauth_flow: OAuth2Flow,
    ) -> Result<Dracoon<Connected>, DracoonClientError> {
        let client = self.client.connect(oauth_flow).await?;

        let mut dracoon = Dracoon {
            client,
            state: PhantomData,
            user_info: None,
            keypair: None,
            encryption_secret: self.encryption_secret,
        };

        if let Some(encryption_secret) = dracoon.encryption_secret.clone() {
            let kp = dracoon.get_user_keypair(&encryption_secret).await?;
            dracoon.encryption_secret = None;
            dracoon.keypair = Some(kp);
            drop(encryption_secret)
        }

        Ok(dracoon)
    }

    pub fn get_authorize_url(&mut self) -> String {
        self.client.get_authorize_url()
    }
}

impl Dracoon<Connected> {
    pub fn build_api_url(&self, url_part: &str) -> Url {
        self.client
            .get_base_url()
            .join(url_part)
            .expect("Correct base url")
    }

    pub async fn get_auth_header(&self) -> Result<String, DracoonClientError> {
        self.client.get_auth_header().await
    }

    pub fn get_base_url(&self) -> &Url {
        self.client.get_base_url()
    }

    pub fn get_refresh_token(&self) -> String {
        self.client.get_refresh_token()
    }

    pub async fn get_user_info(&mut self) -> Result<&UserAccount, DracoonClientError> {
        if let Some(ref user_info) = self.user_info {
            return Ok(user_info);
        }

        let user_info = self.get_user_account().await?;
        self.user_info = Some(user_info);
        Ok(self.user_info.as_ref().expect("Just set user info"))
    }

    pub async fn get_keypair(
        &self,
    ) -> Result<&PlainUserKeyPairContainer, DracoonClientError> {
        if let Some(ref keypair) = self.keypair {
            return Ok(keypair);
        }

        Err(DracoonClientError::MissingEncryptionSecret)

    }
}
