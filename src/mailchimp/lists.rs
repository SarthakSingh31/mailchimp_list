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

    pub async fn add_merge_field(
        &self,
        token: &Token,
        name: impl AsRef<str>,
    ) -> worker::Result<MergeField> {
        let body = serde_json::json!({
            "name": name.as_ref(),
            "type": "url",
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

    pub async fn set_member_merge_field(
        &self,
        token: &Token,
        field_name: impl AsRef<str>,
        member_email_id: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> worker::Result<()> {
        let body = serde_json::json!({
            "merge_fields": {
                field_name.as_ref(): value.as_ref()
            },
        })
        .to_string();

        let resp = token
            .fetch(
                format!("lists/{}/members/{}", self.0, member_email_id.as_ref()).as_str(),
                [],
                Method::Patch,
                Some(body.into()),
            )
            .await?;

        assert!(resp.status_code() < 300);

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct MergeField {
    pub tag: String,
}
