mod session;

use session::Session;
use worker::{Request, Response};

#[derive(serde::Deserialize)]
pub struct Campaign {
    pub id: String,
    pub title: String,
    pub members: Vec<String>,
}

#[worker::event(fetch)]
async fn main(req: Request, env: worker::Env, _ctx: worker::Context) -> worker::Result<Response> {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();

    worker::Router::new()
        // Returns the index page
        .get_async("/", |_req, ctx| async move {
            Response::from_html(
                include_str!("index.html")
                    .replace("{LOGIN_URL}", Session::login_url(&ctx.env).as_str()),
            )
        })
        .get_async(Session::AUTH_CALLBACK, |req, ctx| async move {
            if let Some((_, code)) = req.url()?.query_pairs().find(|(key, _)| key == "code") {
                let session = Session::try_from(&ctx.env)?;
                let id = session.register_session(&*code).await?;

                Response::from_html(
                    include_str!("callback.html").replace("{SESSION_ID}", id.to_string().as_str()),
                )
            } else {
                Response::error("Code query param missing in callback", 400)
            }
        })
        .get_async("/validate_session", |req, ctx| async move {
            if let Some((_, session_id)) = req
                .url()?
                .query_pairs()
                .find(|(key, _)| key == "session_id")
            {
                let session = Session::try_from(&ctx.env)?;

                if session.validate(&*session_id).await? {
                    Response::ok("Valid Session Code")
                } else {
                    Response::error("Invalid Session Code", 401)
                }
            } else {
                Response::error("Missing code query param", 400)
            }
        })
        .get_async("/get_campaigns", |req, ctx| async move {
            let session_id = req
                .headers()
                .get("session-id")?
                .expect("Each request must embed the auth code");

            let session = Session::try_from(&ctx.env)?;
            let token = session.access_token(session_id).await?;

            token.fetch::<&str, &str>("campaigns", []).await
        })
        .get_async("/get_members/:list_id", |req, ctx| async move {
            let Some(list_id) = ctx.param("list_id") else {
                return Response::error("Missing list id", 400);
            };
            let session_id = req
                .headers()
                .get("session-id")?
                .expect("Each request must embed the auth code");

            let session = Session::try_from(&ctx.env)?;
            let token = session.access_token(session_id).await?;

            token
                .fetch::<&str, &str>(format!("lists/{list_id}/members").as_str(), [])
                .await
        })
        .post_async("/sync", |req, ctx| async move {
            let session_id = req
                .headers()
                .get("session-id")?
                .expect("Each request must embed the auth code");

            let session = Session::try_from(&ctx.env)?;

            session.sync(&session_id).await?;

            Response::ok("Synced")
        })
        .run(req, env)
        .await
}
