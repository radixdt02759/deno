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
  // Track which variables came from which files
  file_variables: HashMap<String, HashMap<String, String>>, // file_path -> (variable_name -> value)
  // Track all loaded variables and their sources
  loaded_variables: HashMap<String, String>, // variable_name -> variable value
  // Track all loaded variables and their sources
  unused_variables: HashMap<String, String>, // variable_name -> variable value
  // Track original env vars that existed before we started
  original_env: HashMap<String, String>,
}

impl EnvManagerInner {
  fn new() -> Self {
    // Capture the original environment state
    let original_env: HashMap<String, String> = env::vars().collect();

    Self {
      file_variables: HashMap::new(),
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
    let path_str = file_path.as_ref().to_string_lossy().to_string();

    // Check if file exists
    if !file_path.as_ref().exists() {
      // self._unload_env_file_inner(&mut inner, &file_path)?;
      inner.file_variables.remove(&path_str);
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
        let mut current_file_vars = HashMap::new();
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
              current_file_vars.insert(key.clone(), value.clone());
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
        inner.file_variables.insert(path_str, current_file_vars);
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
    unused_vars: &HashMap<String, String>,
    log_level: Option<log::Level>,
  ) {
    let inner = self.inner.lock().unwrap();
    // Find variables that are loaded but no longer present in any file

    for (var_name, _) in unused_vars {
      // Check if this variable was originally set by us (not from original env)
      if !inner.original_env.contains_key(&var_name.clone()) {
        // Remove the variable from environment
        // SAFETY: We're removing environment variables that we previously set.
        // var_name is a valid UTF-8 string that we tracked when loading the env file.
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
        // Restore original value
        let original_value = inner
          .original_env
          .get(&var_name.clone())
          .map(|s| s.as_str())
          .unwrap_or("");
        // SAFETY: We're restoring environment variables to their original values.
        // Both var_name and original_value are valid UTF-8 strings from the original environment.
        unsafe {
          env::set_var(&var_name.clone(), original_value);
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

  /// Internal helper for unloading (to avoid double-locking)
  fn _unload_env_file_inner<P: AsRef<Path>>(
    &self,
    inner: &mut EnvManagerInner,
    file_path: P,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let path_str: String = file_path.as_ref().to_string_lossy().to_string();
    if let Some(variables) = inner.file_variables.remove(&path_str) {
      for (var_name, value) in variables {
        // Only unload variables that were loaded from this specific file
        if let Some(source_file) = inner.loaded_variables.get(&var_name) {
          if source_file == &path_str {
            // Restore original value or remove entirely
            if let Some(original_value) = inner.original_env.get(&var_name) {
              // SAFETY: We're restoring environment variables to their original values.
              // Both var_name and original_value are valid UTF-8 strings from the original environment.
              unsafe {
                env::set_var(&var_name, original_value);
              }
            } else {
              // Only remove the variable if its current value is the same as when we set it
              match env::var(&var_name) {
                Ok(current_value) => {
                  if current_value.as_str() == value {
                    // SAFETY: We're removing environment variables that we previously set.
                    // var_name is a valid UTF-8 string that we tracked when loading the env file.
                    unsafe {
                      env::remove_var(&var_name);
                    }
                  }
                }
                Err(_) => {
                  // If the variable doesn't exist, nothing to do
                }
              }
            }
            // Remove from loaded_variables tracking since this file is being unloaded
            inner.loaded_variables.remove(&var_name);
          }
        }
      }
    }

    Ok(())
  }

  /// Load multiple env files in order (earlier files take precedence over later ones)
  pub fn load_env_variables_from_env_files<P: AsRef<Path>>(
    &self,
    file_paths: Option<&Vec<P>>,
    log_level: Option<log::Level>,
  ) -> usize {
    let Some(env_file_names) = file_paths else {
      return 0;
    };

    let mut total_loaded = 0;
    let mut inner = self.inner.lock().unwrap();
    // let old_variables = std::mem::take(&mut inner.loaded_variables);
    inner.unused_variables = inner.loaded_variables.clone();
    inner.loaded_variables = HashMap::new();

    for env_file_name in env_file_names.iter().rev() {
      match self.load_env_file(env_file_name, log_level) {
        Ok(count) => {
          total_loaded += count;
        }
        Err(e) => {
          // Log critical errors but continue processing other files
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

    // Clean up variables that are no longer present in any file
    self._cleanup_removed_variables(&inner.unused_variables, log_level);

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
