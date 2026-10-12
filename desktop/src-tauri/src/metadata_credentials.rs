use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "com.hevc-rebuild-studio";
const TMDB_ACCOUNT: &str = "tmdb-api-read-access-token";
const TVDB_ACCOUNT: &str = "tvdb-project-credentials";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmdbCredentialStatus {
    pub(crate) configured: bool,
    pub(crate) secure_store_available: bool,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvdbCredentialStatus {
    pub(crate) configured: bool,
    pub(crate) subscriber_pin_configured: bool,
    pub(crate) secure_store_available: bool,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TvdbCredentials {
    pub(crate) api_key: String,
    pub(crate) subscriber_pin: Option<String>,
}

fn tmdb_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, TMDB_ACCOUNT)
        .map_err(|_| "Could not open the operating-system credential store.".to_string())
}

fn tvdb_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, TVDB_ACCOUNT)
        .map_err(|_| "Could not open the operating-system credential store.".to_string())
}

pub(crate) fn tmdb_status() -> TmdbCredentialStatus {
    if Entry::store_status().is_err() {
        return TmdbCredentialStatus {
            configured: false,
            secure_store_available: false,
            message: Some(
                "The operating-system credential store is unavailable. Unlock or enable the desktop keyring and try again.".into(),
            ),
        };
    }

    match tmdb_entry().and_then(|entry| match entry.get_password() {
        Ok(token) if !token.trim().is_empty() => Ok(true),
        Ok(_) | Err(KeyringError::NoEntry) => Ok(false),
        Err(_) => Err(
            "Could not read the saved TMDB credential from the operating-system keyring.".into(),
        ),
    }) {
        Ok(configured) => TmdbCredentialStatus {
            configured,
            secure_store_available: true,
            message: None,
        },
        Err(message) => TmdbCredentialStatus {
            configured: false,
            secure_store_available: false,
            message: Some(message),
        },
    }
}

pub(crate) fn save_tmdb_token(token: &str) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
        return Err("Enter a valid TMDB API Read Access Token (up to 4,096 characters).".into());
    }
    tmdb_entry()?
        .set_password(token)
        .map_err(|_| "Could not save the TMDB token to the operating-system keyring.".into())
}

pub(crate) fn read_tmdb_token() -> Result<String, String> {
    match tmdb_entry()?.get_password() {
        Ok(token) if !token.trim().is_empty() => Ok(token),
        Ok(_) | Err(KeyringError::NoEntry) => {
            Err("No TMDB token is saved. Add your API Read Access Token in Settings.".into())
        }
        Err(_) => {
            Err("Could not read the saved TMDB token from the operating-system keyring.".into())
        }
    }
}

pub(crate) fn remove_tmdb_token() -> Result<(), String> {
    match tmdb_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err("Could not remove the TMDB token from the operating-system keyring.".into()),
    }
}

pub(crate) fn tvdb_status() -> TvdbCredentialStatus {
    if Entry::store_status().is_err() {
        return TvdbCredentialStatus {
            configured: false,
            subscriber_pin_configured: false,
            secure_store_available: false,
            message: Some(
                "The operating-system credential store is unavailable. Unlock or enable the desktop keyring and try again.".into(),
            ),
        };
    }

    match tvdb_entry().and_then(|entry| match entry.get_password() {
        Ok(serialized) => serde_json::from_str::<TvdbCredentials>(&serialized).map_err(|_| {
            "The saved TheTVDB credentials could not be read. Replace them in Settings.".to_string()
        }),
        Err(KeyringError::NoEntry) => Ok(TvdbCredentials {
            api_key: String::new(),
            subscriber_pin: None,
        }),
        Err(_) => {
            Err("Could not read TheTVDB credentials from the operating-system keyring.".into())
        }
    }) {
        Ok(credentials) => TvdbCredentialStatus {
            configured: !credentials.api_key.trim().is_empty(),
            subscriber_pin_configured: credentials
                .subscriber_pin
                .as_deref()
                .is_some_and(|pin| !pin.trim().is_empty()),
            secure_store_available: true,
            message: None,
        },
        Err(message) => TvdbCredentialStatus {
            configured: false,
            subscriber_pin_configured: false,
            secure_store_available: false,
            message: Some(message),
        },
    }
}

pub(crate) fn save_tvdb_credentials(
    api_key: &str,
    subscriber_pin: &str,
    access_confirmed: bool,
) -> Result<(), String> {
    if !access_confirmed {
        return Err(
            "Confirm that this project has valid TheTVDB API access before saving its key.".into(),
        );
    }
    let api_key = api_key.trim();
    let subscriber_pin = subscriber_pin.trim();
    if api_key.is_empty() || api_key.len() > 512 || api_key.chars().any(char::is_control) {
        return Err("Enter a valid TheTVDB project API key (up to 512 characters).".into());
    }
    if subscriber_pin.len() > 128 || subscriber_pin.chars().any(char::is_control) {
        return Err(
            "The TheTVDB subscriber PIN is too long or contains invalid characters.".into(),
        );
    }
    let credentials = TvdbCredentials {
        api_key: api_key.into(),
        subscriber_pin: (!subscriber_pin.is_empty()).then(|| subscriber_pin.into()),
    };
    let serialized = serde_json::to_string(&credentials)
        .map_err(|_| "Could not prepare TheTVDB credentials for secure storage.".to_string())?;
    tvdb_entry()?
        .set_password(&serialized)
        .map_err(|_| "Could not save TheTVDB credentials to the operating-system keyring.".into())
}

pub(crate) fn read_tvdb_credentials() -> Result<TvdbCredentials, String> {
    let serialized = tvdb_entry()?.get_password().map_err(|error| match error {
        KeyringError::NoEntry => {
            String::from("No TheTVDB project API key is saved. Add it in Settings.")
        }
        _ => String::from("Could not read TheTVDB credentials from the operating-system keyring."),
    })?;
    let credentials: TvdbCredentials = serde_json::from_str(&serialized).map_err(|_| {
        "The saved TheTVDB credentials could not be read. Replace them in Settings.".to_string()
    })?;
    if credentials.api_key.trim().is_empty() {
        return Err("No TheTVDB project API key is saved. Add it in Settings.".into());
    }
    Ok(credentials)
}

pub(crate) fn remove_tvdb_credentials() -> Result<(), String> {
    match tvdb_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => {
            Err("Could not remove TheTVDB credentials from the operating-system keyring.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn token_validation_never_accepts_empty_or_control_text() {
        assert!(super::save_tmdb_token("\n").is_err());
        assert!(super::save_tmdb_token("token\nline").is_err());
        assert!(super::save_tmdb_token(&"x".repeat(4097)).is_err());
    }
}
