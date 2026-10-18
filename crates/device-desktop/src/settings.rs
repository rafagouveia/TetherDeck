//! Local, non-secret UI preferences. Loaded once; saved only by explicit UI changes.
use std::{fs, io::Write, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Dark,
    Light,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LanguagePreference {
    System,
    Portuguese,
    English,
    Spanish,
}
#[derive(Clone)]
pub struct Preferences {
    pub theme: ThemePreference,
    pub language: LanguagePreference,
    pub reduced_motion: bool,
    pub auto_discover: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            language: LanguagePreference::System,
            reduced_motion: false,
            auto_discover: true,
        }
    }
}
fn path() -> Result<PathBuf, String> {
    let root = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .ok_or_else(|| {
            "O diretório de preferências do aplicativo não está disponível.".to_owned()
        })?;
    Ok(PathBuf::from(root)
        .join("TetherDeck")
        .join("preferences.json"))
}
impl Preferences {
    pub fn load() -> (Self, Option<String>) {
        let path = match path() {
            Ok(path) => path,
            Err(error) => return (Self::default(), Some(error)),
        };
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!("Não foi possível ler as preferências: {error}")),
                );
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&data) {
            Ok(value) => value,
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!("Arquivo de preferências inválido: {error}")),
                );
            }
        };
        if !value.is_object() {
            return (
                Self::default(),
                Some("Arquivo de preferências inválido: formato inesperado.".into()),
            );
        }
        (
            Self {
                theme: match value["theme"].as_str() {
                    Some("dark") => ThemePreference::Dark,
                    Some("light") => ThemePreference::Light,
                    _ => ThemePreference::System,
                },
                language: match value["language"].as_str() {
                    Some("pt") => LanguagePreference::Portuguese,
                    Some("en") => LanguagePreference::English,
                    Some("es") => LanguagePreference::Spanish,
                    _ => LanguagePreference::System,
                },
                reduced_motion: value["reduced_motion"].as_bool().unwrap_or(false),
                auto_discover: value["auto_discover"].as_bool().unwrap_or(true),
            },
            None,
        )
    }
    pub fn save(&self) -> Result<(), String> {
        let path = path()?;
        let parent = path
            .parent()
            .ok_or_else(|| "Diretório de preferências inválido.".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let data = serde_json::json!({"version":1,
            "theme":match self.theme { ThemePreference::System => "system", ThemePreference::Dark => "dark", ThemePreference::Light => "light" },
            "language":match self.language { LanguagePreference::System => "system", LanguagePreference::Portuguese => "pt", LanguagePreference::English => "en", LanguagePreference::Spanish => "es" },
            "reduced_motion":self.reduced_motion,"auto_discover":self.auto_discover});
        let bytes = serde_json::to_vec_pretty(&data).map_err(|error| error.to_string())?;
        let pending = path.with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
        let mut file = fs::File::create(&pending).map_err(|error| error.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        replace_file(&pending, &path)
    }
}

pub(crate) fn replace_file(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // Both paths live in the same app directory. Replace only after the
        // complete preferences have been written and flushed to disk.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x1 | 0x8) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target).map_err(|error| error.to_string())
    }
}
