use serde_json::Value;
use worker::Method;

use super::Token;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Member {
    pub email_address: String,
    pub full_name: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Members {
    pub members: Vec<Member>,
    pub total_items: usize,
}

pub struct List(pub String);

impl List {
    pub async fn fetch_members(
        &self,
        token: &Token,
        after_time: Option<impl AsRef<str>>,
    ) -> worker::Result<Members> {
        let endpoint = format!("lists/{}/members", self.0);

        let mut members = Members {
            members: Vec::default(),
            total_items: 0,
        };

        loop {
            let resp = token
                .fetch(
                    &endpoint,
                    after_time
                        .as_ref()
                        .map(|t| ("since_last_changed", t.as_ref()))
                        .into_iter()
                        .chain(
                            [
                                ("count", "1000"),
                                ("offset", members.members.len().to_string().as_str()),
                            ]
                            .into_iter(),
                        ),
                    Method::Get,
                    None,
                )
                .await?
                .json::<Members>()
                .await?;

            members.members.extend(resp.members);

            if members.members.len() == resp.total_items {
                break;
            }
        }

        Ok(members)
    }

    pub async fn get_or_add_merge_field(
        &self,
        token: &Token,
        name: impl AsRef<str>,
    ) -> worker::Result<MergeField> {
        #[derive(Debug, serde::Deserialize)]
        struct MergeFields {
            merge_fields: Vec<MergeField>,
        }

        let fields = token
            .fetch(
                format!("lists/{}/merge-fields", self.0).as_str(),
                [],
                Method::Get,
                None,
            )
            .await?
            .json::<MergeFields>()
            .await?
            .merge_fields;

        if let Some(field) = fields.into_iter().find(|field| field.name == name.as_ref()) {
            return Ok(field);
        }

        let body = serde_json::json!({
            "name": name.as_ref(),
            "type": "text",
            "tag": name.as_ref(),
            "required": false,
            "public": false,
        })
        .to_string();

        token
            .fetch(
                format!("lists/{}/merge-fields", self.0).as_str(),
                [],
                Method::Post,
                Some(body.into()),
            )
            .await?
            .json()
            .await
    }

    pub async fn set_member_merge_field_batch(
        &self,
        token: &Token,
        values: impl IntoIterator<Item = (impl AsRef<str>, Vec<(impl AsRef<str>, impl AsRef<str>)>)>,
    ) -> worker::Result<()> {
        let mut operations = Vec::default();

        for (member_email_id, values) in values {
            let uri = format!("lists/{}/members/{}", self.0, member_email_id.as_ref());
            let mut merge_fields = serde_json::Map::new();
            for (key, value) in values {
                merge_fields.insert(
                    key.as_ref().to_string(),
                    Value::String(value.as_ref().to_string()),
                );
            }
            let body = serde_json::json!({
                "merge_fields": Value::Object(merge_fields),
            });

            operations.push(serde_json::json!({
                "method": "PATCH",
                "path": uri,
                "params": {},
                "body": body.to_string()
            }));
        }

        let operations = serde_json::json!({
            "operations": Value::Array(operations),
        });

        token
            .fetch(
                "batches",
                [],
                Method::Post,
                Some(operations.to_string().into()),
            )
            .await?;

        Ok(())
    }

    pub async fn install_webhook(
        &self,
        token: &Token,
        url: impl AsRef<str>,
    ) -> worker::Result<String> {
        let body = serde_json::json!({
            "url": url.as_ref(),
            "events": {
                "subscribe": true,
                "profile": true
            },
            "sources": {
                "user": true,
                "admin": true,
                "api": true
            },
        });

        #[derive(serde::Deserialize)]
        struct Webhook {
            id: String,
        }

        let webhook: Webhook = token
            .fetch(
                format!("lists/{}/webhooks", self.0).as_str(),
                [],
                Method::Post,
                Some(body.to_string().into()),
            )
            .await?
            .json()
            .await?;

        Ok(webhook.id)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct MergeField {
    pub tag: String,
    pub name: String,
}
