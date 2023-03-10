use std::collections::{HashMap, HashSet};

use serde_json::Value;
use worker::{wasm_bindgen::JsValue, Env, Fetch, Headers, Method, Request, RequestInit, Response};

use crate::mailchimp::{campaign::MailChimpCampaign, lists::List, Token};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct User {
    #[serde(rename = "Id")]
    pub id: u64,
    #[serde(rename = "Username")]
    pub name: String,
    #[serde(rename = "Email")]
    pub email: String,
    #[serde(rename = "LastSynced")]
    pub last_synced: Option<i64>,
}

pub struct Session {
    db: worker::D1Database,
    client_id: String,
    client_secret: String,
    webhook_uri: url::Url,
    redirect_uri: url::Url,
}

impl Session {
    pub const BINDING: &'static str = "MailchimpDB";
    pub const AUTH_CALLBACK: &'static str = "/auth/token";
    pub const WEBHOOK_CALLBACK: &'static str = "/webhook";
    const AUTH_URL: &'static str = "https://login.mailchimp.com/oauth2/";
    const TOKEN_URL: &'static str = "https://login.mailchimp.com/oauth2/token";
    const METADATA_URL: &'static str = "https://login.mailchimp.com/oauth2/metadata";

    pub fn login_url(env: &Env) -> url::Url {
        let mut url = url::Url::parse(Self::AUTH_URL)
            .expect("Failed to parse AUTH_URL")
            .join("authorize")
            .expect("Failed to build login url");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("response_type", "code");
            query.append_pair("client_id", Self::client_id_from_env(env).as_str());
            query.append_pair("redirect_uri", Self::redirect_uri_from_env(env).as_str());
        }

        url
    }

    pub async fn register_session(
        &self,
        code: impl std::fmt::Display,
    ) -> worker::Result<uuid::Uuid> {
        let id = uuid::Uuid::new_v4();

        let mut headers = Headers::default();
        headers.append("Content-Type", "application/x-www-form-urlencoded")?;

        let req = Request::new_with_init(
            Self::TOKEN_URL,
            &RequestInit {
                body: Some(
                    format!(
                        "grant_type=authorization_code&client_id={}&client_secret={}&redirect_uri={}&code={}",
                        self.client_id, self.client_secret, self.redirect_uri, code
                    )
                    .into(),
                ),
                headers,
                method: Method::Post,
                ..Default::default()
            },
        )?;

        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        struct InnerToken {
            access_token: String,
        }

        let mut resp = Fetch::Request(req).send().await?;
        let token: InnerToken = resp.json().await?;

        let mut headers = Headers::default();
        headers.append(
            "Authorization",
            format!("OAuth {}", token.access_token).as_str(),
        )?;

        let req = Request::new_with_init(
            Self::METADATA_URL,
            &RequestInit {
                headers,
                ..Default::default()
            },
        )?;

        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        struct LoginMetadata {
            email: String,
        }

        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        struct Metadata {
            user_id: u64,
            accountname: String,
            dc: String,
            login: LoginMetadata,
        }

        let mut resp = Fetch::Request(req).send().await?;
        let metadata: Metadata = resp.json().await?;

        if let Err(_) = self.get_user(metadata.user_id).await {
            self.db
                .prepare(format!(
                    "INSERT INTO Users VALUES ({}, ?, ?);",
                    metadata.user_id
                ))
                .bind(&[metadata.accountname.into(), metadata.login.email.into()])?
                .all()
                .await?;
        }

        self.db
            .prepare(format!(
                "INSERT INTO UserSessions VALUES (?, {}, ?, ?);",
                metadata.user_id
            ))
            .bind(&[
                id.to_string().into(),
                token.access_token.into(),
                metadata.dc.into(),
            ])?
            .all()
            .await?;

        Ok(id)
    }

    pub async fn validate(&self, session_id: impl Into<JsValue>) -> worker::Result<bool> {
        let count = self
            .db
            .prepare("SELECT AccessToken, Dc FROM UserSessions WHERE Id = ?;")
            .bind(&[session_id.into()])?
            .all()
            .await?
            .results::<Token>()?
            .len();

        Ok(count == 1)
    }

    pub async fn access_token(&self, session_id: impl Into<JsValue>) -> worker::Result<Token> {
        let token = self
            .db
            .prepare("SELECT AccessToken, Dc FROM UserSessions WHERE Id = ?;")
            .bind(&[session_id.into()])?
            .all()
            .await?;
        let tokens: Vec<Token> = token.results()?;

        if let Some(token) = tokens.first() {
            Ok(token.clone())
        } else {
            Err(worker::Error::RustError(
                "Failed to find a token for this session".into(),
            ))
        }
    }

    pub async fn access_token_from_list_id(
        &self,
        list_id: impl Into<JsValue>,
    ) -> worker::Result<Token> {
        let tokens = self
            .db
            .prepare("SELECT AccessToken, Dc FROM UserSessions WHERE UserId = (SELECT Id FROM Users WHERE Id = (SELECT UserId FROM Lists WHERE Id = ?));")
            .bind(&[list_id.into()])?
            .all()
            .await?
            .results::<Token>()?;

        if let Some(token) = tokens.first() {
            Ok(token.clone())
        } else {
            Err(worker::Error::RustError(
                "Failed to find a session for this list_id. This probably happened because a user session was deleted from the db".into(),
            ))
        }
    }

    pub async fn get_existing_campaign_merge_fields_in(
        &self,
        campaigns: HashSet<String>,
    ) -> worker::Result<HashMap<String, (String, String)>> {
        #[derive(serde::Deserialize)]
        struct DbCampaign {
            #[serde(rename = "Id")]
            id: String,
            #[serde(rename = "VideoTag")]
            video_tag: String,
            #[serde(rename = "ImageTag")]
            image_tag: String,
        }

        let campaigns = campaigns
            .into_iter()
            .map(|campaign| format!("'{}'", campaign))
            .collect::<Vec<_>>()
            .join(",");

        Ok(self
            .db
            .prepare(format!(
                "SELECT Id, VideoTag, ImageTag FROM Campaigns WHERE Id in ({});",
                campaigns
            ))
            .bind(&[])?
            .all()
            .await?
            .results::<DbCampaign>()?
            .into_iter()
            .map(|campaign| (campaign.id, (campaign.video_tag, campaign.image_tag)))
            .collect())
    }

    /// Adds a campaign to the
    pub async fn add_campaign_to_table(
        &self,
        campaign: &MailChimpCampaign,
        session_id: impl Into<JsValue> + Copy,
        video_tag: impl Into<JsValue>,
        image_tag: impl Into<JsValue>,
    ) -> worker::Result<()> {
        #[derive(serde::Deserialize)]
        struct DbSession {
            #[serde(rename = "UserId")]
            user_id: u64,
        }

        let sessions = self
            .db
            .prepare("SELECT UserId FROM UserSessions WHERE Id = ?;")
            .bind(&[session_id.into()])?
            .all()
            .await?
            .results::<DbSession>()?;
        let session = sessions.first().expect("Failed to find session");

        let token = self.access_token(session_id).await?;

        // Populate the lists table if it did not exist
        if self
            .db
            .prepare("SELECT * FROM Lists WHERE Id = ?;")
            .bind(&[campaign.recipients.list_id.as_str().into()])?
            .all()
            .await?
            .results::<Value>()?
            .first()
            .is_none()
        {
            let list = List(campaign.recipients.list_id.clone());
            let webhook_id = list
                .install_webhook(&token, self.webhook_uri.as_str())
                .await?;

            self.db
                .prepare(format!(
                    "INSERT INTO Lists VALUES (?, {}, ?);",
                    session.user_id
                ))
                .bind(&[
                    campaign.recipients.list_id.as_str().into(),
                    webhook_id.as_str().into(),
                ])?
                .all()
                .await?;
            let members = list
                .fetch_members(&token, Option::<&str>::None)
                .await?
                .members
                .into_iter()
                .map(|member| {
                    format!(
                        "('{}', '{}', '{}')",
                        member.email_address, member.full_name, campaign.recipients.list_id
                    )
                })
                .collect::<Vec<_>>()
                .join(",");

            if !members.is_empty() {
                self.db
                    .exec(format!("INSERT INTO Members VALUES {};", members).as_str())
                    .await?;
            }
        }

        // Populate the campaign table if it did not exist
        self.db
            .prepare(format!(
                "INSERT INTO Campaigns VALUES (?, ?, ?, {}, ?, ?);",
                session.user_id
            ))
            .bind(&[
                campaign.id.as_str().into(),
                campaign.settings.title.as_str().into(),
                campaign.recipients.list_id.as_str().into(),
                video_tag.into(),
                image_tag.into(),
            ])?
            .all()
            .await?;

        Ok(())
    }

    pub async fn populate_merge_fields(
        &self,
        session_id: impl Into<JsValue> + Copy,
        campaign_id: &str,
    ) -> worker::Result<Response> {
        let token = self.access_token(session_id).await?;

        let campaign = MailChimpCampaign::get(&token, campaign_id).await?;
        let list = List(campaign.recipients.list_id.clone());

        let video_field = list
            .get_or_add_merge_field(&token, &format!("Video/{}", campaign.id))
            .await?;
        let image_field = list
            .get_or_add_merge_field(&token, &format!("Image/{}", campaign.id))
            .await?;
        self.add_campaign_to_table(&campaign, session_id, &video_field.tag, &image_field.tag)
            .await?;

        let values = list
            .fetch_members(&token, Option::<&str>::None)
            .await?
            .members
            .into_iter()
            .map(|member| {
                (
                    member.email_address,
                    vec![
                        (&video_field.tag, "vimeo.com/226053498"),
                        (&image_field.tag, "s3.amazonaws.com/creare-websites-wpms-legacy/wp-content/uploads/sites/32/2016/03/01200959/canstockphoto22402523-arcos-creator.com_-1024x1024.jpg"),
                    ],
                )
            });
        list.set_member_merge_field_batch(&token, values).await?;

        Response::from_json(&serde_json::json!({
            "video_tag": video_field.tag,
            "image_tag": image_field.tag,
        }))
    }

    pub async fn subscribe_member(
        &self,
        token: &Token,
        email: &str,
        name: impl Into<JsValue>,
        list_id: &str,
    ) -> worker::Result<()> {
        self.db
            .prepare("INSERT INTO Members VALUES (?, ?, ?);")
            .bind(&[email.into(), name.into(), list_id.into()])?
            .all()
            .await?;

        #[derive(serde::Deserialize)]
        struct DbCampaign {
            #[serde(rename = "VideoTag")]
            video_tag: String,
            #[serde(rename = "ImageTag")]
            image_tag: String,
        }

        let list = List(list_id.to_owned());

        let values = self
            .db
            .prepare("SELECT VideoTag, ImageTag FROM Campaigns WHERE ListId = ?;")
            .bind(&[list_id.into()])?
            .all()
            .await?
            .results::<DbCampaign>()?
            .into_iter()
            .map(|campaign| {
                (
                    email,
                    vec![
                        (campaign.video_tag, "vimeo.com/226053498"),
                        (campaign.image_tag, "s3.amazonaws.com/creare-websites-wpms-legacy/wp-content/uploads/sites/32/2016/03/01200959/canstockphoto22402523-arcos-creator.com_-1024x1024.jpg"),
                    ],
                )
            });

        list.set_member_merge_field_batch(&token, values).await?;

        Ok(())
    }

    pub async fn update_member(
        &self,
        token: &Token,
        email: &str,
        name: &str,
        list_id: &str,
    ) -> worker::Result<()> {
        #[derive(serde::Deserialize)]
        struct DbMember {
            #[serde(rename = "FullName")]
            name: String,
        }

        let members = self
            .db
            .prepare("SELECT FullName FROM Members WHERE EmailId = ?;")
            .bind(&[email.into()])?
            .all()
            .await?
            .results::<DbMember>()?;

        let Some(member) = members.first() else {
            return Err(worker::Error::RustError("Failed to find the user will email id".into()));
        };

        let list = List(list_id.to_owned());

        #[derive(serde::Deserialize)]
        struct DbCampaign {
            #[serde(rename = "VideoTag")]
            video_tag: String,
            #[serde(rename = "ImageTag")]
            image_tag: String,
        }

        if member.name != name {
            self.db
                .prepare("UPDATE Members SET FullName = ? WHERE  EmailId = ?;")
                .bind(&[name.into(), email.into()])?
                .all()
                .await?;

            let values = self
                .db
                .prepare("SELECT VideoTag, ImageTag FROM Campaigns WHERE ListId = ?;")
                .bind(&[list_id.into()])?
                .all()
                .await?
                .results::<DbCampaign>()?
                .into_iter()
                .map(|campaign| {
                    (
                        email,
                        vec![
                            (campaign.video_tag, "vimeo.com/226053498"),
                            (campaign.image_tag, "s3.amazonaws.com/creare-websites-wpms-legacy/wp-content/uploads/sites/32/2016/03/01200959/canstockphoto22402523-arcos-creator.com_-1024x1024.jpg"),
                        ],
                    )
                });

            list.set_member_merge_field_batch(&token, values).await?;
        }

        Ok(())
    }

    async fn get_user(&self, user_id: impl std::fmt::Display) -> worker::Result<User> {
        // BUGFIX: Binding the query normall was causing issues
        let query = format!("SELECT * FROM Users WHERE Id = {};", user_id);

        let mut users: Vec<User> = self.db.prepare(query).bind(&[])?.all().await?.results()?;

        users.pop().ok_or(worker::Error::RustError(
            "Failed to find user for user_id".into(),
        ))
    }

    fn client_id_from_env(env: &Env) -> String {
        env.secret("MAILCHIMP_CLIENT_ID")
            .expect("Failed to find MAILCHIMP_CLIENT_ID secret")
            .to_string()
    }

    fn client_secret_from_env(env: &Env) -> String {
        env.secret("MAILCHIMP_CLIENT_SECRET")
            .expect("Failed to find MAILCHIMP_CLIENT_SECRET secret")
            .to_string()
    }

    fn redirect_uri_from_env(env: &Env) -> url::Url {
        env.secret("MAILCHIMP_BASE_URI")
            .expect("Failed to find MAILCHIMP_BASE_URI secret")
            .to_string()
            .parse::<url::Url>()
            .expect("MAILCHIMP_BASE_URI is not a valid uri")
            .join(Self::AUTH_CALLBACK)
            .expect("Failed to join the token endpoint")
    }

    fn webhook_uri_from_env(env: &Env) -> url::Url {
        env.secret("MAILCHIMP_BASE_URI")
            .expect("Failed to find MAILCHIMP_BASE_URI secret")
            .to_string()
            .parse::<url::Url>()
            .expect("MAILCHIMP_BASE_URI is not a valid uri")
            .join(Self::WEBHOOK_CALLBACK)
            .expect("Failed to join the token endpoint")
    }
}

impl TryFrom<&Env> for Session {
    type Error = worker::Error;

    fn try_from(env: &Env) -> Result<Self, Self::Error> {
        Ok(Session {
            db: env.d1(Self::BINDING)?,
            client_id: Self::client_id_from_env(&env),
            client_secret: Self::client_secret_from_env(&env),
            webhook_uri: Self::webhook_uri_from_env(&env),
            redirect_uri: Self::redirect_uri_from_env(&env),
        })
    }
}
