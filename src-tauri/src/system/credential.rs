use keyring_core::Entry;

const SERVICE_NAME: &str = "ai-desktop-pet";

pub fn set_api_key(profile_id: &str, api_key: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, profile_id).map_err(|e| e.to_string())?;
    entry.set_password(api_key).map_err(|e| e.to_string())
}

pub fn get_api_key(profile_id: &str) -> Option<String> {
    let entry = Entry::new(SERVICE_NAME, profile_id).ok()?;
    entry.get_password().ok()
}

pub fn delete_api_key(profile_id: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, profile_id).map_err(|e| e.to_string())?;
    let _ = entry.delete_credential();
    Ok(())
}
