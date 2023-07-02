use chrono::{DateTime, Utc};
use reqwest::{Client, Url};
use std::marker::PhantomData;
use tracing::{debug, error};

use base64::{
    self, alphabet,
    engine::{self, general_purpose},
    Engine,
};

pub mod errors;
pub mod models;

pub use models::*;

use crate::{
    auth::models::{
        OAuth2AuthCodeFlow, OAuth2PasswordFlow, OAuth2TokenResponse, OAuth2TokenRevoke,
    },
    constants::{DRACOON_TOKEN_REVOKE_URL, DRACOON_TOKEN_URL, TOKEN_TYPE_HINT_ACCESS_TOKEN},
};

use self::{errors::DracoonClientError, models::OAuth2RefreshTokenFlow};
use super::constants::{APP_USER_AGENT, TOKEN_TYPE_HINT_REFRESH_TOKEN};

/// represents the possible `OAuth2` flows
pub enum OAuth2Flow {
    PasswordFlow(String, String),
    AuthCodeFlow(String),
    RefreshToken(String),
}

/// connected state of [DracoonClient]
#[derive(Debug, Clone)]
pub struct Connected;
/// disconnected state of [DracoonClient]
#[derive(Debug, Clone)]
pub struct Disconnected;

/// represents a connection to DRACOON (`OAuth2` tokens)
#[derive(Debug, Clone)]
pub struct Connection {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u32,
    pub connected_at: DateTime<Utc>,
}

#[derive(Clone)]
/// represents the DRACOON client (stateful)
pub struct DracoonClient<State = Disconnected> {
    base_url: Url,
    redirect_uri: Option<Url>,
    client_id: String,
    client_secret: String,
    pub http: Client,
    connection: Option<Connection>,
    connected: PhantomData<State>,
}

/// Builder for the [DracoonClient] struct.
#[derive(Default)]
pub struct DracoonClientBuilder {
    base_url: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
}

impl DracoonClientBuilder {
    /// Creates a new [DracoonClientBuilder]
    pub fn new() -> Self {
        Self {
            base_url: None,
            redirect_uri: None,
            client_id: None,
            client_secret: None,
        }
    }

    /// Sets the base url for the DRACOON instance
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Sets the redirect uri for the OAuth2 flow (required for the auth code flow)
    pub fn with_redirect_uri(mut self, redirect_uri: impl Into<String>) -> Self {
        self.redirect_uri = Some(redirect_uri.into());
        self
    }

    /// Sets the client id for the OAuth2 flow
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Sets the client secret for the OAuth2 flow
    pub fn with_client_secret(mut self, client_secret: impl Into<String>) -> Self {
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Builds the [DracoonClient] struct - returns an error if any of the required fields are missing
    pub fn build(self) -> Result<DracoonClient<Disconnected>, DracoonClientError> {
        let http = Client::builder().user_agent(APP_USER_AGENT).build()?;

        let Some(base_url) = self.base_url.clone() else {
            error!("Missing base url");
            return Err(DracoonClientError::MissingBaseUrl)
        };

        let base_url = Url::parse(&base_url)?;

        let Some(client_id) = self.client_id else {
            error!("Missing client id");
            return Err(DracoonClientError::MissingClientId)
        };

        let Some(client_secret) = self.client_secret else {
            error!("Missing client secret");
            return Err(DracoonClientError::MissingClientSecret)
        };

        let redirect_uri = match self.redirect_uri {
            Some(url) => Url::parse(&url)?,
            None => Url::parse(&format!(
                "{}/oauth/callback",
                self.base_url.expect("missing base url already checked")
            ))?,
        };

        Ok(DracoonClient {
            base_url,
            redirect_uri: Some(redirect_uri),
            client_id,
            client_secret,
            connection: None,
            connected: PhantomData,
            http,
        })
    }
}

/// [DracoonClient] implementation for Disconnected state
impl DracoonClient<Disconnected> {
    /// Connects to DRACOON using any of the supported OAuth2 flows
    pub async fn connect(
        self,
        oauth_flow: OAuth2Flow,
    ) -> Result<DracoonClient<Connected>, DracoonClientError> {
        let connection = match oauth_flow {
            OAuth2Flow::PasswordFlow(username, password) => {
                debug!("Connecting with password flow");
                self.connect_password_flow(&username, &password).await?
            }
            OAuth2Flow::AuthCodeFlow(code) => {
                debug!("Connecting with auth code flow");
                self.connect_authcode_flow(&code).await?
            }
            OAuth2Flow::RefreshToken(token) => {
                debug!("Connecting with refresh token flow");
                self.connect_refresh_token(&token).await?
            }
        };

        Ok(DracoonClient {
            client_id: self.client_id,
            client_secret: self.client_secret,
            connection: Some(connection),
            base_url: self.base_url,
            redirect_uri: self.redirect_uri,
            connected: PhantomData,
            http: self.http,
        })
    }

    /// returns client id and client secret bas64 encoded for the basic auth header
    fn client_credentials(&self) -> String {
        const B64_URLSAFE: engine::GeneralPurpose =
            engine::GeneralPurpose::new(&alphabet::URL_SAFE, general_purpose::NO_PAD);
        let client_credentials = format!("{}:{}", &self.client_id, &self.client_secret);

        B64_URLSAFE.encode(client_credentials)
    }

    /// Returns the authorize url for the OAuth2 auth code flow
    pub fn get_authorize_url(&mut self) -> String {
        let default_redirect = self
            .base_url
            .join("oauth/callback")
            .expect("Correct base url");
        let redirect_uri = self
            .redirect_uri
            .as_ref()
            .unwrap_or(&default_redirect)
            .clone();

        self.redirect_uri = Some(redirect_uri.clone());

        let mut authorize_url = self
            .base_url
            .join("oauth/authorize")
            .expect("Correct base url");
        let authorize_url = authorize_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri.as_ref())
            .append_pair("scope", "all")
            .finish();

        authorize_url.to_string()
    }

    /// Returns the token url for any OAuth2 flow
    fn get_token_url(&self) -> Url {
        self.base_url
            .join(DRACOON_TOKEN_URL)
            .expect("Correct base url")
    }

    /// Connects to DRACOON using the password flow
    async fn connect_password_flow(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Connection, DracoonClientError> {
        let token_url = self.get_token_url();

        let auth = OAuth2PasswordFlow::new(username, password);
        let auth_header = format!("Basic {}", self.client_credentials());

        let res = self
            .http
            .post(token_url)
            .header("Authorization", auth_header)
            .form(&auth)
            .send()
            .await.map_err(|err| {
                error!("Error connecting with password flow: {}", err);
                err
            })?;
        Ok(OAuth2TokenResponse::from_response(res).await?.into())
    }

    /// Connects to DRACOON using the auth code flow
    async fn connect_authcode_flow(&self, code: &str) -> Result<Connection, DracoonClientError> {
        let token_url = self.get_token_url();

        let auth = OAuth2AuthCodeFlow::new(
            &self.client_id,
            &self.client_secret,
            code,
            self.redirect_uri
                .as_ref()
                .expect("redirect uri is set")
                .as_str(),
        );

        let res = self.http.post(token_url).form(&auth).send().await.map_err(|err| {
            error!("Error connecting with auth code flow: {}", err);
            err
        })?;
        Ok(OAuth2TokenResponse::from_response(res).await?.into())
    }

    /// Connects to DRACOON using the refresh token flow
    async fn connect_refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<Connection, DracoonClientError> {
        let token_url = self.get_token_url();

        let auth = OAuth2RefreshTokenFlow::new(&self.client_id, &self.client_secret, refresh_token);

        let res = self.http.post(token_url).form(&auth).send().await.map_err(|err| {
           error!("Error connecting with refresh token flow: {}", err);
              err 
        })?;
        Ok(OAuth2TokenResponse::from_response(res).await?.into())
    }
}

/// `DracoonClient` implementation for Connected state
impl DracoonClient<Connected> {
    /// disconnects the client and optionally revokes the access and refresh token
    /// access token is revoked by default, refresh token is not revoked by default
    pub async fn disconnect(
        self,
        revoke_access_token: Option<bool>,
        revoke_refresh_token: Option<bool>,
    ) -> Result<DracoonClient<Disconnected>, DracoonClientError> {
        let revoke_access_token = revoke_access_token.unwrap_or(true);
        let revoke_refresh_token = revoke_refresh_token.unwrap_or(false);

        if revoke_access_token {
            debug!("Revoking access token");
            self.revoke_acess_token().await?;
        }

        if revoke_refresh_token {
            debug!("Revoking refresh token");
            self.revoke_refresh_token().await?;
        }

        Ok(DracoonClient {
            client_id: self.client_id,
            client_secret: self.client_secret,
            connection: None,
            base_url: self.base_url,
            redirect_uri: self.redirect_uri,
            connected: PhantomData,
            http: self.http,
        })
    }
    
    /// Returns the base url of the DRACOON instance
    pub fn get_base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns the token url for any OAuth2 flow
    fn get_token_url(&self) -> Url {
        self.base_url
            .join(DRACOON_TOKEN_URL)
            .expect("Correct base url")
    }

    /// Revokes the access token
    async fn revoke_acess_token(&self) -> Result<(), DracoonClientError> {
        let access_token = self
            .connection
            .as_ref()
            .expect("Connected client has a connection")
            .access_token
            .clone();

        let api_url = self
            .base_url
            .join(DRACOON_TOKEN_REVOKE_URL)
            .expect("Correct base url");

        let auth = OAuth2TokenRevoke::new(
            &self.client_id,
            &self.client_secret,
            TOKEN_TYPE_HINT_ACCESS_TOKEN,
            &access_token,
        );

        self.http.post(api_url).form(&auth).send().await?;

        Ok(())
    }

    /// Revokes the refresh token
    async fn revoke_refresh_token(&self) -> Result<(), DracoonClientError> {
        let refresh_token = self
            .connection
            .as_ref()
            .expect("Connected client has a connection")
            .refresh_token
            .clone();

        let api_url = self
            .base_url
            .join(DRACOON_TOKEN_REVOKE_URL)
            .expect("Correct base url");

        let auth = OAuth2TokenRevoke::new(
            &self.client_id,
            &self.client_secret,
            TOKEN_TYPE_HINT_REFRESH_TOKEN,
            &refresh_token,
        );

        self.http.post(api_url).form(&auth).send().await?;

        Ok(())
    }

    /// Fetches new tokens using available refresh token from the current connection
    async fn connect_refresh_token(&self) -> Result<Connection, DracoonClientError> {
        let token_url = self.get_token_url();

        let connection = self
            .connection
            .as_ref()
            .expect("Connected client has a connection");

        let auth = OAuth2RefreshTokenFlow::new(
            &self.client_id,
            &self.client_secret,
            connection.refresh_token.as_str(),
        );

        let res = self.http.post(token_url).form(&auth).send().await?;
        Ok(OAuth2TokenResponse::from_response(res).await?.into())
    }

    /// Returns the necessary token header for any API call that requires authentication in DRACOON
    pub async fn get_auth_header(&self) -> Result<String, DracoonClientError> {
        if !self.check_access_token_validity() {
            self.connect_refresh_token().await?;
        }

        Ok(format!(
            "Bearer {}",
            self.connection
                .as_ref()
                .expect("Connected client has a connection")
                .access_token
        ))
    }

    /// Returns the refresh token
    pub fn get_refresh_token(&self) -> &str {
        self.connection
            .as_ref()
            .expect("Connected client has a connection")
            .refresh_token
            .as_str()
    }

    /// Checks if the access token is still valid
    fn check_access_token_validity(&self) -> bool {
        let connection = self
            .connection
            .as_ref()
            .expect("Connected client has a connection");

        let now = Utc::now();

        (now - connection.connected_at).num_seconds() < connection.expires_in.into()
    }
}

#[cfg(test)]
mod tests {
    use tokio_test::assert_ok;

    use super::*;

    fn get_test_client(url: &str) -> DracoonClient<Disconnected> {
        DracoonClientBuilder::new()
            .with_base_url(url)
            .with_client_id("client_id")
            .with_client_secret("client_secret")
            .build()
            .expect("valid client config")
    }

    #[test]
    fn test_auth_code_authentication() {
        let mut mock_server = mockito::Server::new();
        let base_url = mock_server.url();

        let auth_res = include_str!("./tests/auth_ok.json");

        let auth_mock = mock_server
            .mock("POST", "/oauth/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(auth_res)
            .create();

        let dracoon = DracoonClientBuilder::new()
            .with_base_url(base_url)
            .with_client_id("client_id")
            .with_client_secret("client_secret")
            .build()
            .expect("valid client config");

        let auth_code = OAuth2Flow::AuthCodeFlow("hello world".to_string());

        let res = tokio_test::block_on(dracoon.connect(auth_code));

        auth_mock.assert();
        assert_ok!(&res);

        assert!(res.unwrap().connection.is_some());
    }

    #[test]
    fn test_refresh_token_authentication() {
        let mut mock_server = mockito::Server::new();
        let base_url = mock_server.url();

        let auth_res = include_str!("./tests/auth_ok.json");

        let auth_mock = mock_server
            .mock("POST", "/oauth/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(auth_res)
            .create();

        let dracoon = get_test_client(base_url.as_str());

        let refresh_token_auth = OAuth2Flow::RefreshToken("hello world".to_string());

        let res = tokio_test::block_on(dracoon.connect(refresh_token_auth));

        auth_mock.assert();
        assert_ok!(&res);

        assert!(res.as_ref().unwrap().connection.is_some());

        let access_token = res
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .access_token
            .clone();
        let refresh_token = res
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .refresh_token
            .clone();
        let expires_in = res.unwrap().connection.unwrap().expires_in;

        assert_eq!(access_token, "access_token");
        assert_eq!(refresh_token, "refresh_token");
        assert_eq!(expires_in, 3600);
    }

    #[test]
    fn test_auth_error_handling() {
        let mut mock_server = mockito::Server::new();
        let base_url = mock_server.url();

        let auth_res = include_str!("./tests/auth_error.json");

        let auth_mock = mock_server
            .mock("POST", "/oauth/token")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(auth_res)
            .create();

        let dracoon = get_test_client(base_url.as_str());

        let auth_code = OAuth2Flow::AuthCodeFlow("hello world".to_string());

        let res = tokio_test::block_on(dracoon.connect(auth_code));

        auth_mock.assert();

        assert!(res.is_err());
    }

    #[test]
    fn test_get_auth_header() {
        let mut mock_server = mockito::Server::new();
        let base_url = mock_server.url();

        let auth_res = include_str!("./tests/auth_ok.json");

        let auth_mock = mock_server
            .mock("POST", "/oauth/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(auth_res)
            .create();

        let dracoon = get_test_client(base_url.as_str());
        let refresh_token_auth = OAuth2Flow::RefreshToken("hello world".to_string());

        let res = tokio_test::block_on(dracoon.connect(refresh_token_auth));
        let connected_client = res.unwrap();

        let access_token = tokio_test::block_on(connected_client.get_auth_header()).unwrap();

        auth_mock.assert();
        assert_eq!(access_token, "Bearer access_token");
    }

    #[test]
    fn test_get_token_url() {
        let base_url = "https://dracoon.team";

        let dracoon = get_test_client(base_url);

        let token_url = dracoon.get_token_url();

        assert_eq!(token_url.as_str(), "https://dracoon.team/oauth/token");
    }

    #[test]
    fn test_get_base_url() {
        let mut mock_server = mockito::Server::new();
        let base_url = mock_server.url();

        let auth_res = include_str!("./tests/auth_ok.json");

        let auth_mock = mock_server
            .mock("POST", "/oauth/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(auth_res)
            .create();

        let dracoon = get_test_client(&base_url);
        let dracoon = tokio_test::block_on(
            dracoon.connect(OAuth2Flow::AuthCodeFlow("hello world".to_string())),
        )
        .unwrap();

        let base_url = dracoon.get_base_url();

        auth_mock.assert();
        assert_eq!(base_url.as_str(), format!("{}/", mock_server.url()));
    }

    #[test]
    fn test_get_refresh_token() {
        let mut mock_server = mockito::Server::new();
        let base_url = mock_server.url();

        let auth_res = include_str!("./tests/auth_ok.json");

        let auth_mock = mock_server
            .mock("POST", "/oauth/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(auth_res)
            .create();

        let dracoon = get_test_client(&base_url);
        let dracoon = tokio_test::block_on(
            dracoon.connect(OAuth2Flow::AuthCodeFlow("hello world".to_string())),
        )
        .unwrap();

        let refresh_token = dracoon.get_refresh_token();

        auth_mock.assert();
        assert_eq!(refresh_token, "refresh_token");
    }
}
