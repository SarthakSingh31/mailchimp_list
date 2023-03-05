use worker::{wasm_bindgen::JsValue, Env, Fetch, Headers, Method, Request, RequestInit, Response};

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

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Token {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "Dc")]
    dc: String,
}

impl Token {
    const API_URL: &'static str = "https://<dc>.api.mailchimp.com/3.0/";

    fn endpoint(&self, uri: &str) -> url::Url {
        Self::API_URL
            .replace("<dc>", &self.dc)
            .parse::<url::Url>()
            .expect("Failed to parse api url")
            .join(uri)
            .expect("Failed to build endpoint url")
    }

    pub async fn fetch<K: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: &str,
        params: impl IntoIterator<Item = (K, V)>,
    ) -> worker::Result<worker::Response> {
        let mut headers = Headers::default();
        headers.append(
            "Authorization",
            format!("Bearer {}", self.access_token).as_str(),
        )?;

        let init = RequestInit {
            headers,
            ..Default::default()
        };
        let mut uri = self.endpoint(uri);
        {
            let mut query_params = uri.query_pairs_mut();
            for (key, value) in params {
                query_params.append_pair(key.as_ref(), value.as_ref());
            }
        }

        Fetch::Request(Request::new_with_init(uri.as_str(), &init)?)
            .send()
            .await
    }
}

pub struct Session {
    db: worker::D1Database,
    client_id: String,
    client_secret: String,
    redirect_uri: url::Url,
}

impl Session {
    pub const BINDING: &'static str = "MailchimpDB";
    pub const AUTH_CALLBACK: &'static str = "/auth/token";
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
                    "INSERT INTO Users VALUES ({}, ?, ?, NULL);",
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

    pub async fn sync(&self, session_id: impl Into<JsValue> + Copy) -> worker::Result<Response> {
        let Some(user_id) = ({
            #[derive(serde::Deserialize)]
            struct UserSession {
                #[serde(rename = "UserId")]
                user_id: u64,
            }

            let mut user_sessions: Vec<UserSession> = self
                .db
                .prepare("SELECT UserId FROM UserSessions WHERE Id = ?")
                .bind(&[session_id.into()])?
                .all()
                .await?
                .results()?;

            user_sessions.pop().map(|user_session| user_session.user_id)
        }) else {
            return Err(worker::Error::RustError("Failed to find a session for this session id".into()));
        };

        let user = self.get_user(user_id).await?;
        let since_create_time = user.last_synced.map(|t| {
            time::OffsetDateTime::from_unix_timestamp(t)
                .expect("Failed to convert timestamp to time")
                .format(&time::format_description::well_known::Iso8601::DEFAULT)
                .expect("Failed to format timestamp")
        });

        let token = self.access_token(session_id).await?;

        #[derive(Debug, serde::Deserialize)]
        struct DbCampaign {
            #[serde(rename = "Id")]
            id: String,
            #[serde(rename = "Title")]
            title: String,
            #[serde(rename = "MemberListId")]
            member_list_id: String,
            #[serde(skip_deserializing)]
            new: bool,
        }
        let mut db_campaigns: Vec<DbCampaign> = self
            .db
            .prepare(format!(
                "SELECT Id, Title, MemberListId FROM Campaigns WHERE UserId = {};",
                user_id
            ))
            .bind(&[])?
            .all()
            .await?
            .results()?;

        // Set the time of last sync at when we start syncing everything
        self.db
            .exec(
                format!(
                    "UPDATE Users SET LastSynced = {} WHERE Id = {};",
                    time::OffsetDateTime::now_utc().unix_timestamp(),
                    user_id
                )
                .as_str(),
            )
            .await?;

        let new_mc_campagins = {
            #[derive(Debug, serde::Deserialize)]
            struct MailChimpRecipients {
                list_id: String,
            }

            #[derive(Debug, serde::Deserialize)]
            struct MailChimpSettings {
                title: String,
            }

            #[derive(Debug, serde::Deserialize)]
            struct MailChimpCampaign {
                id: String,
                recipients: MailChimpRecipients,
                settings: MailChimpSettings,
            }

            #[derive(Debug, serde::Deserialize)]
            struct MailChimpCampaigns {
                campaigns: Vec<MailChimpCampaign>,
                total_items: usize,
            }

            let mut campaigns = Vec::new();

            loop {
                let resp = token
                    .fetch(
                        "campaigns",
                        since_create_time
                            .as_ref()
                            .map(|t| ("since_create_time", t.as_str()))
                            .into_iter()
                            .chain(
                                [
                                    ("count", "1000"),
                                    ("offset", campaigns.len().to_string().as_str()),
                                ]
                                .into_iter(),
                            ),
                    )
                    .await?
                    .json::<MailChimpCampaigns>()
                    .await?;

                campaigns.extend(resp.campaigns);

                if campaigns.len() == resp.total_items {
                    break;
                }
            }

            campaigns
        };

        let mut mc_campaign_insert = Vec::default();

        for mc_campaign in new_mc_campagins {
            mc_campaign_insert.push(format!(
                "('{}', '{}', '{}', {})",
                mc_campaign.id, mc_campaign.settings.title, mc_campaign.recipients.list_id, user_id,
            ));

            db_campaigns.push(DbCampaign {
                id: mc_campaign.id,
                title: mc_campaign.settings.title,
                member_list_id: mc_campaign.recipients.list_id,
                new: true,
            });
        }

        if mc_campaign_insert.len() > 0 {
            self.db
                .exec(
                    format!(
                        "INSERT INTO Campaigns VALUES {};",
                        mc_campaign_insert.join(",")
                    )
                    .as_str(),
                )
                .await?;
        }

        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct Member {
            email_address: String,
            full_name: String,
        }

        #[derive(serde::Serialize)]
        struct NewCampaignData {
            title: String,
            new_members: Vec<Member>,
        }

        let mut new_campaign_data = Vec::new();
        let mut new_members_insert = Vec::new();

        for db_campaign in &db_campaigns {
            let new_members = {
                #[derive(Debug, serde::Deserialize)]
                struct Members {
                    members: Vec<Member>,
                    total_items: usize,
                }

                let params = if db_campaign.new {
                    None
                } else {
                    since_create_time
                        .as_ref()
                        .map(|t| ("since_last_changed", t.as_str()))
                };

                let mut members = Vec::new();
                let endpoint = format!("lists/{}/members", db_campaign.member_list_id);

                loop {
                    let resp = token
                        .fetch(
                            &endpoint,
                            params.into_iter().chain(
                                [
                                    ("count", "1000"),
                                    ("offset", members.len().to_string().as_str()),
                                ]
                                .into_iter(),
                            ),
                        )
                        .await?
                        .json::<Members>()
                        .await?;

                    members.extend(resp.members);

                    if members.len() == resp.total_items {
                        break;
                    }
                }

                members
            };

            for member in &new_members {
                new_members_insert.push(format!(
                    "('{}', '{}', '{}')",
                    member.email_address, member.full_name, db_campaign.id
                ));
            }

            new_campaign_data.push(NewCampaignData {
                title: db_campaign.title.clone(),
                new_members,
            });
        }

        if new_members_insert.len() > 0 {
            self.db
                .exec(
                    format!(
                        "INSERT INTO Members VALUES {};",
                        new_members_insert.join(",")
                    )
                    .as_str(),
                )
                .await?;
        }

        Response::from_json(&new_campaign_data)
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
}

impl TryFrom<&Env> for Session {
    type Error = worker::Error;

    fn try_from(env: &Env) -> Result<Self, Self::Error> {
        Ok(Session {
            db: env.d1(Self::BINDING)?,
            client_id: Self::client_id_from_env(&env),
            client_secret: Self::client_secret_from_env(&env),
            redirect_uri: Self::redirect_uri_from_env(&env),
        })
    }
}
