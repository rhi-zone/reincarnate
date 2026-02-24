/// Persistence platform trait — key-value string storage.
///
/// The platform interface stores raw strings. Serialization (JSON, bincode, etc.)
/// is the API shim's responsibility, not the platform's. This keeps the platform
/// contract free of serde or any encoding dependency.
///
/// Implementations: browser localStorage + OPFS, filesystem, IndexedDB, etc.
pub trait Persistence {
    /// Write a string value under key.
    fn save(&mut self, key: &str, data: &str);

    /// Read a string value by key. Returns None if not found.
    fn load(&self, key: &str) -> Option<String>;

    /// Remove a key from storage.
    fn remove(&mut self, key: &str);
}
