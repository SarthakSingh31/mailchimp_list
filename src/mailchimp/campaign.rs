use worker::Method;

use super::Token;

pub const BASE_URL: &'static str = "campaigns";

#[derive(Debug, serde::Deserialize)]
pub struct MailChimpRecipients {
    pub list_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct MailChimpSettings {
    pub title: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct MailChimpCampaign {
    pub id: String,
    pub recipients: MailChimpRecipients,
    pub settings: MailChimpSettings,
}

impl MailChimpCampaign {
    pub async fn get(token: &Token, campaign_id: impl AsRef<str>) -> worker::Result<Self> {
        token
            .fetch(
                format!("{BASE_URL}/{}", campaign_id.as_ref()).as_str(),
                [],
                Method::Get,
                None,
            )
            .await?
            .json()
            .await
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct MailChimpCampaigns {
    pub campaigns: Vec<MailChimpCampaign>,
    pub total_items: usize,
}

impl MailChimpCampaigns {
    pub async fn get_all(
        token: &Token,
        after_time: Option<impl AsRef<str>>,
    ) -> worker::Result<Self> {
        let mut campaigns = MailChimpCampaigns {
            campaigns: Vec::default(),
            total_items: 0,
        };

        loop {
            let resp = token
                .fetch(
                    BASE_URL,
                    after_time
                        .as_ref()
                        .map(|t| ("since_create_time", t.as_ref()))
                        .into_iter()
                        .chain(
                            [
                                ("count", "1000"),
                                ("offset", campaigns.campaigns.len().to_string().as_str()),
                            ]
                            .into_iter(),
                        ),
                    Method::Get,
                    None,
                )
                .await?
                .json::<MailChimpCampaigns>()
                .await?;

            campaigns.campaigns.extend(resp.campaigns);
            campaigns.total_items = resp.total_items;

            if campaigns.campaigns.len() == resp.total_items {
                break;
            }
        }

        Ok(campaigns)
    }
}
