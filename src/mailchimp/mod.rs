pub mod campaign;
pub mod lists;

use worker::{wasm_bindgen::JsValue, Fetch, Headers, Method, Request, RequestInit};

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

    pub async fn fetch(
        &self,
        uri: &str,
        params: impl IntoIterator<Item = (&str, &str)>,
        method: Method,
        body: Option<JsValue>,
    ) -> worker::Result<worker::Response> {
        let mut headers = Headers::default();
        headers.append(
            "Authorization",
            format!("Bearer {}", self.access_token).as_str(),
        )?;

        let init = RequestInit {
            headers,
            method,
            body,
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
