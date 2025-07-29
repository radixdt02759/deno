// Copyright 2018-2025 the Deno authors. MIT license.

use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use deno_terminal::colors;

#[derive(Debug, Clone)]
struct EnvManagerInner {
  // Track all loaded variables and their values
  loaded_variables: HashMap<String, String>, // variable_name -> variable value
  // Track variables that are no longer present in any loaded file
  unused_variables: HashMap<String, String>, // variable_name -> variable value
  // Track original env vars that existed before we started
  original_env: HashMap<String, String>,
}

impl EnvManagerInner {
  fn new() -> Self {
    // Capture the original environment state
    let original_env: HashMap<String, String> = env::vars().collect();

    Self {
      loaded_variables: HashMap::new(),
      unused_variables: HashMap::new(),
      original_env,
    }
  }
}

#[derive(Debug, Clone)]
pub struct EnvManager {
  inner: Arc<Mutex<EnvManagerInner>>,
}

// Global singleton instance
static ENV_MANAGER: OnceLock<EnvManager> = OnceLock::new();

impl EnvManager {
  /// Get the global singleton instance
  pub fn instance() -> &'static EnvManager {
    ENV_MANAGER.get_or_init(|| EnvManager {
      inner: Arc::new(Mutex::new(EnvManagerInner::new())),
    })
  }
  ///
  /// # Arguments
  /// * `file_path` - Path to the .env file
  /// * `log_level` - Optional log level to control error message visibility
  ///
  /// # Returns
  /// * `Ok(usize)` - Number of variables successfully loaded
  /// * `Err(Box<dyn std::error::Error>)` - Critical errors that prevent loading
  pub fn load_env_file<P: AsRef<Path>>(
    &self,
    file_path: P,
    log_level: Option<log::Level>,
  ) -> Result<usize, Box<dyn std::error::Error>> {
    let mut inner = self.inner.lock().unwrap();
    self.load_env_file_inner(file_path, log_level, &mut inner)
  }

  /// Internal method that accepts an already-acquired lock to avoid deadlocks
  fn load_env_file_inner<P: AsRef<Path>>(
    &self,
    file_path: P,
    log_level: Option<log::Level>,
    inner: &mut EnvManagerInner,
  ) -> Result<usize, Box<dyn std::error::Error>> {
    let path_str = file_path.as_ref().to_string_lossy().to_string();

    // Check if file exists
    if !file_path.as_ref().exists() {
      // Only show warning if logging is enabled
      #[allow(clippy::print_stderr)]
      if log_level.map(|l| l >= log::Level::Info).unwrap_or(true) {
        eprintln!(
          "{} The environment file specified '{}' was not found.",
          colors::yellow("Warning"),
          path_str
        );
      }
      return Ok(0);
    }

    let mut loaded_count = 0;

    match dotenvy::from_path_iter(file_path.as_ref()) {
      Ok(iter) => {
        for item in iter {
          match item {
            Ok((key, value)) => {
              // Check if this variable is already loaded from a previous file
              if inner.loaded_variables.contains_key(&key) {
                // Variable already exists from a previous file, skip it
                #[allow(clippy::print_stderr)]
                if log_level.map(|l| l >= log::Level::Debug).unwrap_or(false) {
                  eprintln!(
                    "{} Variable '{}' already loaded from '{}', skipping value from '{}'",
                    colors::yellow("Debug"),
                    key,
                    inner
                      .loaded_variables
                      .get(&key)
                      .unwrap_or(&"unknown".to_string()),
                    path_str
                  );
                }
                continue;
              }

              // Set the environment variable
              // SAFETY: We're setting environment variables with valid UTF-8 strings
              // from the .env file. Both key and value are guaranteed to be valid strings.
              unsafe {
                env::set_var(&key, &value);
              }

              // Track this variable
              inner.loaded_variables.insert(key.clone(), value.clone());
              if inner.unused_variables.contains_key(&key) {
                inner.unused_variables.remove(&key);
              }
              loaded_count += 1;
            }
            Err(e) => {
              // Handle parsing errors with detailed messages
              #[allow(clippy::print_stderr)]
              if log_level.map(|l| l >= log::Level::Info).unwrap_or(true) {
                match e {
                  dotenvy::Error::LineParse(line, index) => eprintln!(
                    "{} Parsing failed within the specified environment file: {} at index: {} of the value: {}",
                    colors::yellow("Warning"),
                    path_str,
                    index,
                    line
                  ),
                  dotenvy::Error::Io(_) => eprintln!(
                    "{} The `--env-file` flag was used, but the environment file specified '{}' was not found.",
                    colors::yellow("Warning"),
                    path_str
                  ),
                  dotenvy::Error::EnvVar(_) => eprintln!(
                    "{} One or more of the environment variables isn't present or not unicode within the specified environment file: {}",
                    colors::yellow("Warning"),
                    path_str
                  ),
                  _ => eprintln!(
                    "{} Unknown failure occurred with the specified environment file: {}",
                    colors::yellow("Warning"),
                    path_str
                  ),
                }
              }
            }
          }
        }
      }
      Err(e) => {
        // This is a critical error - file exists but can't be read
        return Err(format!("Failed to read {}: {}", path_str, e).into());
      }
    }

    Ok(loaded_count)
  }

  /// Clean up variables that are no longer present in any loaded file
  fn _cleanup_removed_variables(
    &self,
    inner: &mut EnvManagerInner,
    log_level: Option<log::Level>,
  ) {
    for var_name in inner.unused_variables.keys() {
      if !inner.original_env.contains_key(var_name) {
        unsafe {
          env::remove_var(&var_name);
        }

        #[allow(clippy::print_stderr)]
        if log_level.map(|l| l >= log::Level::Debug).unwrap_or(false) {
          eprintln!(
            "{} Variable '{}' removed from environment as it's no longer present in any loaded file",
            colors::yellow("Debug"),
            var_name
          );
        }
      } else {
        let original_value = inner.original_env.get(var_name).unwrap();
        unsafe {
          env::set_var(&var_name, original_value);
        }

        #[allow(clippy::print_stderr)]
        if log_level.map(|l| l >= log::Level::Debug).unwrap_or(false) {
          eprintln!(
            "{} Variable '{}' restored to original value as it's no longer present in any loaded file",
            colors::yellow("Debug"),
            var_name
          );
        }
      }
    }
  }

  // Load multiple env files in reverse order (later files take precedence over earlier ones)
  pub fn load_env_variables_from_env_files<P: AsRef<Path>>(
    &self,
    file_paths: Option<&Vec<P>>,
    log_level: Option<log::Level>,
  ) -> usize {
    let Some(env_file_names) = file_paths else {
      return 0;
    };

    let mut inner = self.inner.lock().unwrap();

    inner.unused_variables = std::mem::take(&mut inner.loaded_variables);
    inner.loaded_variables = HashMap::new();

    let mut total_loaded = 0;

    for env_file_name in env_file_names.iter().rev() {
      match self.load_env_file_inner(env_file_name, log_level, &mut inner) {
        Ok(count) => {
          total_loaded += count;
        }
        Err(e) =>
        {
          #[allow(clippy::print_stderr)]
          if log_level.map(|l| l >= log::Level::Info).unwrap_or(true) {
            eprintln!(
              "{} Critical error loading {}: {}",
              colors::yellow("Warning"),
              env_file_name.as_ref().to_string_lossy(),
              e
            );
          }
        }
      }
    }

    self._cleanup_removed_variables(&mut inner, log_level);

    total_loaded
  }
}

pub fn load_env_variables_from_env_files<P: AsRef<Path>>(
  file_paths: &[P],
  flags_log_level: Option<log::Level>,
) -> Result<usize, Box<dyn std::error::Error>> {
  let file_paths_vec: Vec<&P> = file_paths.iter().collect();
  Ok(
    EnvManager::instance().load_env_variables_from_env_files(
      Some(&file_paths_vec),
      flags_log_level,
    ),
  )
}
