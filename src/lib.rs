mod mailchimp;
mod session;

use std::collections::HashMap;

use mailchimp::campaign::MailChimpCampaigns;
use session::Session;
use worker::{Method, Request, Response};

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
        .get_async("/lists", |req, ctx| async move {
            let session_id = req
                .headers()
                .get("session-id")?
                .expect("Each request must embed the auth code");

            let session = Session::try_from(&ctx.env)?;
            let token = session.access_token(session_id).await?;

            token.fetch("lists", [], Method::Get, None).await
        })
        .get_async("/campaigns", |req, ctx| async move {
            let session_id = req
                .headers()
                .get("session-id")?
                .expect("Each request must embed the auth code");

            let session = Session::try_from(&ctx.env)?;
            let token = session.access_token(session_id).await?;

            let campaigns = MailChimpCampaigns::get_all(&token, Option::<&str>::None)
                .await?
                .campaigns;
            let existing_campaigns = session
                .get_existing_campaign_merge_fields_in(
                    campaigns
                        .iter()
                        .map(|campaign| campaign.id.clone())
                        .collect(),
                )
                .await?;

            let campaigns = campaigns
                .into_iter()
                .map(|campaign| {
                    let merge_tags = existing_campaigns.get(&campaign.id).map(|tags| {
                        serde_json::json!({
                            "video_tag": tags.0,
                            "image_tag": tags.1,
                        })
                    });
                    serde_json::json!({
                        "id": campaign.id,
                        "list_id": campaign.recipients.list_id,
                        "title": campaign.settings.title,
                        "merge_tags": merge_tags
                    })
                })
                .collect::<Vec<_>>();

            Response::from_json(&serde_json::json!({
                "campaigns": campaigns,
            }))
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
                .fetch(
                    format!("lists/{list_id}/members").as_str(),
                    [],
                    Method::Get,
                    None,
                )
                .await
        })
        .get_async(Session::WEBHOOK_CALLBACK, |_req, _ctx| async move {
            Response::ok("Hello")
        })
        .post_async(
            "/populate_merge_fields/:campaign_id",
            |req, ctx| async move {
                let Some(campaign_id) = ctx.param("campaign_id") else {
                    return Response::error("Missing list id", 400);
                };
                let session_id = req
                    .headers()
                    .get("session-id")?
                    .expect("Each request must embed the auth code");

                let session = Session::try_from(&ctx.env)?;

                session
                    .populate_merge_fields(&session_id, campaign_id)
                    .await
            },
        )
        .post_async(Session::WEBHOOK_CALLBACK, |mut req, ctx| async move {
            let req = req.bytes().await?;
            let data: Vec<_> = form_urlencoded::parse(&req).collect();
            let data: HashMap<_, _> = data.iter().map(|(key, value)| (&**key, &**value)).collect();

            let Some(email_id) = data.get("data[email]") else {
                return Response::error("Webhook call is missing data[email]", 400);
            };
            let Some(list_id) = data.get("data[list_id]") else {
                return Response::error("Webhook call is missing data[list_id]", 400);
            };
            let Some(fname) = data.get("data[merges][FNAME]") else {
                return Response::error("Webhook call is missing data[merges][FNAME]", 400);
            };
            let Some(lname) = data.get("data[merges][LNAME]") else {
                return Response::error("Webhook call is missing data[merges][LNAME]", 400);
            };

            let session = Session::try_from(&ctx.env)?;
            let token = session.access_token_from_list_id(*list_id).await?;

            match data.get("type") {
                // A new member subscribed
                Some(&"subscribe") => {
                    session
                        .subscribe_member(&token, *email_id, format!("{fname} {lname}"), *list_id)
                        .await?;

                    Response::ok("added")
                }
                // A member's data has changed
                Some(&"profile") => {
                    session
                        .update_member(&token, *email_id, &format!("{fname} {lname}"), *list_id)
                        .await?;

                    Response::ok("updated")
                }
                _ => Response::error("Unsupported type of webhook call", 400),
            }
        })
        .run(req, env)
        .await
}
