pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod test_support;

pub use codex_app_server_protocol::AuthMode;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelsResponse;
pub use config::ModelsManagerConfig;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response() -> std::result::Result<ModelsResponse, serde_json::Error> {
    let mut response: ModelsResponse = serde_json::from_str(include_str!("../models.json"))?;
    append_codez_model_overrides(&mut response.models);
    Ok(response)
}

fn append_codez_model_overrides(models: &mut Vec<ModelInfo>) {
    for slug in ["deepseek-v4-flash", "deepseek-v4-pro"] {
        if !models.iter().any(|model| model.slug == slug) {
            models.push(crate::model_info::model_info_from_slug(slug));
        }
    }
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    )
}
