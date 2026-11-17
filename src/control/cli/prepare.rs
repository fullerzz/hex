//! A model checked and loaded through a local API's
//! `POST /models/{id}/prepare`: the running Hex's or an owned embedded
//! service's, with progress shown on a terminal's stderr. Hex never
//! downloads models, so a missing file fails asking for one.

use std::time::Duration;

use super::progress;
use crate::control::ControlReply;
use crate::local_api::Endpoint;
use crate::transcription::models::{TranscriptionSelection, definition};

/// The stream sends a keep-alive every 5 s; quieter than this is a stall.
const IDLE: Duration = Duration::from_secs(60);
const MAX_EVENT: usize = 64 * 1024;

/// Checks and loads `selection`'s model, a file already on this computer, in
/// `endpoint`'s service, until it reports `ok`. It never downloads. Nothing about the selected model changes.
pub(super) fn prepare(
    endpoint: &Endpoint,
    selection: &TranscriptionSelection,
) -> Result<(), ControlReply> {
    let model = definition(selection.model);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("language", &selection.language)
        .finish();
    let path = format!("/models/{}/prepare?{query}", selection.model.as_str());
    let mut outcome = None;
    let answer = endpoint.events(&path, IDLE, MAX_EVENT, |event| {
        match event.get("type").and_then(|kind| kind.as_str()) {
            Some("downloading") => {
                let done = event["downloadedBytes"].as_u64().unwrap_or(0);
                let total = event["totalBytes"].as_u64().unwrap_or(0);
                progress::show(&if total > 0 {
                    format!(
                        "Downloading {}… {} of {}",
                        model.name,
                        progress::megabytes(done),
                        progress::megabytes(total)
                    )
                } else {
                    format!("Downloading {}…", model.name)
                });
                false
            }
            Some("verifying") => {
                progress::show(&format!("Checking {}…", model.name));
                false
            }
            Some("loading") => {
                progress::show(&format!("Loading {}…", model.name));
                false
            }
            Some("ok") => {
                outcome = Some(Ok(()));
                true
            }
            Some("error") => {
                let code = event["error"]["code"]
                    .as_str()
                    .unwrap_or("failed")
                    .to_owned();
                outcome = Some(Err(code));
                true
            }
            _ => false,
        }
    });
    progress::clear();
    let name = model.name.as_ref();
    match answer {
        Ok((200, _)) => match outcome {
            Some(Ok(())) => Ok(()),
            Some(Err(code)) => Err(failed(name, &code)),
            None => Err(ControlReply::error(
                "failed",
                format!("Hex stopped preparing {name} before it finished; try again"),
            )),
        },
        Ok((status, body)) => {
            let code = crate::local_api::error_code(status, &body);
            Err(match code.as_str() {
                "prepare-busy" => ControlReply::error(
                    "busy",
                    "Hex is already preparing a model; try again once it finishes",
                ),
                "unknown-model" => ControlReply::error(
                    "unknown-model",
                    format!(
                        "Hex doesn't know `{}`; run `hex model list`",
                        selection.model.as_str()
                    ),
                ),
                "unsupported-model" => ControlReply::error(
                    "model-unavailable",
                    format!("{name} doesn't support `{}`", selection.language),
                ),
                _ => {
                    ControlReply::error("failed", format!("Hex refused to prepare {name} ({code})"))
                }
            })
        }
        Err(error) => Err(ControlReply::error("transport", format!("{error:#}"))),
    }
}

fn failed(name: &str, code: &str) -> ControlReply {
    let what = match code {
        "download-failed" => "couldn't download",
        "verification-failed" => "couldn't verify",
        "load-failed" => "couldn't load",
        "cancelled" => "stopped preparing",
        _ => "couldn't prepare",
    };
    ControlReply::error("failed", format!("Hex {what} {name}; try again"))
}
