use std::collections::{HashMap, HashSet};
use std::env;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use deno_terminal::colors;

#[derive(Debug, Clone)]
struct EnvManagerInner {
    // Track which variables came from which files
    file_variables: HashMap<String, HashSet<String>>, // file_path -> set of variable names
    // Track all loaded variables and their sources
    loaded_variables: HashMap<String, String>, // variable_name -> file_path (source)
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
        ENV_MANAGER.get_or_init(|| {
            EnvManager {
                inner: Arc::new(Mutex::new(EnvManagerInner::new())),
            }
        })
    }

    /// Create a new instance (mainly for testing - prefer using instance())
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EnvManagerInner::new())),
        }
    }

    /// Load environment variables from a .env file
   /// Load environment variables from a .env file with comprehensive error handling
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
    log_level: Option<log::Level>
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut inner = self.inner.lock().unwrap();
    let path_str = file_path.as_ref().to_string_lossy().to_string();
    self._unload_env_file_inner(&mut inner, &file_path)?;
    // Check if file exists
    if !file_path.as_ref().exists() {
        inner.file_variables.remove(&path_str);
        // Only show warning if logging is enabled
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
    let mut current_file_vars = HashSet::new();

    // Use dotenvy to parse the file
    match dotenvy::from_filename(file_path.as_ref()) {
        Ok(_) => {
            // Since from_filename doesn't give us granular control,
            // we need to use dotenvy::from_path_iter for better error handling
            match dotenvy::from_path_iter(file_path.as_ref()) {
                Ok(iter) => {
                    for item in iter {
                        match item {
                            Ok((key, value)) => {
                                // Set the environment variable
                                unsafe { env::set_var(&key, &value); }
                                
                                // Track this variable
                                current_file_vars.insert(key.clone());
                                inner.loaded_variables.insert(key.clone(), path_str.clone());
                                loaded_count += 1;
                                
                                // Optional debug logging
                                if log_level.map(|l| l >= log::Level::Debug).unwrap_or(false) {
                                    println!("Loaded: {}={} (from {})", key, value, path_str);
                                }
                            }
                            Err(e) => {
                                // Handle parsing errors with detailed messages
                                if log_level.map(|l| l >= log::Level::Info).unwrap_or(true) {
                                    match e {
                                        dotenvy::Error::LineParse(line, index) => eprintln!(
                                            "{} Parsing failed within the specified environment file: {} at index: {} of the value: {}",
                                            colors::yellow("Warning"),
                                            path_str,
                                            index,
                                            line
                                        ),
                                        dotenvy::Error::EnvVar(_) => eprintln!(
                                            "{} One or more of the environment variables isn't present or not unicode within the specified environment file: {}",
                                            colors::yellow("Warning"),
                                            path_str
                                        ),
                                        _ => eprintln!(
                                            "{} Failed to parse line in {}: {}",
                                            colors::yellow("Warning"),
                                            path_str,
                                            e
                                        ),
                                    }
                                }
                                // Continue processing other lines instead of failing completely
                            }
                        }
                    }
                }
                Err(e) => {
                    // This is a critical error - file exists but can't be read
                    return Err(format!("Failed to read {}: {}", path_str, e).into());
                }
            }
        }
        Err(error) => {
            // Handle different types of errors with appropriate logging
            if log_level.map(|l| l >= log::Level::Info).unwrap_or(true) {
                match error {
                    dotenvy::Error::LineParse(line, index) => eprintln!(
                        "{} Parsing failed within the specified environment file: {} at index: {} of the value: {}",
                        colors::yellow("Warning"),
                        path_str,
                        index,
                        line
                    ),
                    dotenvy::Error::Io(_) => eprintln!(
                        "{} The environment file specified '{}' was not found.",
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
            // Don't return error for parsing issues, just log them
            // This matches the behavior of your original function
        }
    }

    // Store the variables for this file
    inner.file_variables.insert(path_str, current_file_vars);
    
    Ok(loaded_count)
}

    /// Internal helper for unloading (to avoid double-locking)
    fn _unload_env_file_inner<P: AsRef<Path>>(&self, inner: &mut EnvManagerInner, file_path: P) -> Result<(), Box<dyn std::error::Error>> {
        let path_str: String = file_path.as_ref().to_string_lossy().to_string();
        println!("Unloading env file: {}", path_str);
        if let Some(variables) = inner.file_variables.remove(&path_str) {
            for var_name in variables {
                // Remove from loaded_variables tracking
                inner.loaded_variables.remove(&var_name);
                
                // Restore original value or remove entirely
                if let Some(original_value) = inner.original_env.get(&var_name) {
                    unsafe { env::set_var(&var_name, original_value); }
                    println!("Restored original: {}={}", var_name, original_value);
                } else {
                    // Only remove the variable if its current value is the same as when we set it (i.e., not changed by user/code)
                    match env::var(&var_name) {
                        Ok(current_value) => {
                            // If the variable is not present in original_env, we set it, so only remove if unchanged
                            if current_value.as_str() == inner.loaded_variables.get(&var_name).unwrap_or(&"".to_string())     {
                                unsafe { env::remove_var(&var_name); }
                            }
                        }
                        Err(_) => {
                            // If the variable doesn't exist, nothing to do
                        }
                    }
                    println!("Removed: {}", var_name);
                }
            }
        }
        
        Ok(())
    }

    /// Unload environment variables from a specific file
    pub fn unload_env_file<P: AsRef<Path>>(&self, file_path: P) -> Result<(), Box<dyn std::error::Error>> {
        let mut inner = self.inner.lock().unwrap();
        self._unload_env_file_inner(&mut inner, file_path)
    }

    /// Reload a specific env file (useful for file watching)
    pub fn reload_env_file<P: AsRef<Path>>(&self, file_path: P) -> Result<usize, Box<dyn std::error::Error>> {
        println!("Reloading env file: {}", file_path.as_ref().display());
        self.load_env_file(file_path, None)
    }

    /// Load multiple env files in order (later files override earlier ones)
    pub fn load_env_variables_from_env_files<P: AsRef<Path>>(
        &self,
        file_paths: Option<&Vec<P>>,
        log_level: Option<log::Level>,
    ) -> usize {
        let Some(env_file_names) = file_paths else {
            return 0;
        };
        
        let mut total_loaded = 0;
        
        // Process files in reverse order (matches original behavior)
        for env_file_name in env_file_names.iter().rev() {
            match self.load_env_file(env_file_name, log_level) {
                Ok(count) => {
                    total_loaded += count;
                }
                Err(e) => {
                    // Log critical errors but continue processing other files
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
        
        total_loaded
    }
    /// Get all currently loaded variables and their sources
    pub fn get_loaded_variables(&self) -> HashMap<String, String> {
        let inner = self.inner.lock().unwrap();
        inner.loaded_variables.clone()
    }

    /// Get variables loaded from a specific file
    pub fn get_variables_from_file<P: AsRef<Path>>(&self, file_path: P) -> Option<HashSet<String>> {
        let inner = self.inner.lock().unwrap();
        let path_str = file_path.as_ref().to_string_lossy().to_string();
        inner.file_variables.get(&path_str).cloned()
    }

    /// Print current environment state
    pub fn print_status(&self) {
        let inner = self.inner.lock().unwrap();
        println!("\n=== Environment Manager Status ===");
        println!("Loaded files: {}", inner.file_variables.len());
        
        for (file_path, variables) in &inner.file_variables {
            println!("  {}: {} variables", file_path, variables.len());
            for var in variables {
                if let Ok(value) = env::var(var) {
                    println!("    {}={}", var, value);
                }
            }
        }
        
        println!("Total managed variables: {}", inner.loaded_variables.len());
        println!("==================================\n");
    }

    /// Get the count of loaded files
    pub fn loaded_files_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.file_variables.len()
    }

    /// Get the count of managed variables
    pub fn managed_variables_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.loaded_variables.len()
    }

    /// Clean up all managed environment variables
    pub fn cleanup(&self) {
        let mut inner = self.inner.lock().unwrap();
        println!("Cleaning up all managed environment variables...");
        
        let file_paths: Vec<String> = inner.file_variables.keys().cloned().collect();
        for file_path in file_paths {
            let _ = self._unload_env_file_inner(&mut inner, Path::new(&file_path));
        }
    }

    /// Check if a variable is managed by this instance
    pub fn is_managed_variable(&self, var_name: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.loaded_variables.contains_key(var_name)
    }

    /// Get the source file of a managed variable
    pub fn get_variable_source(&self, var_name: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner.loaded_variables.get(var_name).cloned()
    }
}

// Convenience functions for global access
pub fn load_env_file<P: AsRef<Path>>(file_path: P) -> Result<usize, Box<dyn std::error::Error>> {
    EnvManager::instance().load_env_file(file_path, None)
}

pub fn load_env_variables_from_env_files<P: AsRef<Path>>(file_paths: &[P]) -> Result<usize, Box<dyn std::error::Error>> {
    let file_paths_vec: Vec<&P> = file_paths.iter().collect();
    Ok(EnvManager::instance().load_env_variables_from_env_files(Some(&file_paths_vec), None))
}

pub fn unload_env_file<P: AsRef<Path>>(file_path: P) -> Result<(), Box<dyn std::error::Error>> {
    EnvManager::instance().unload_env_file(file_path)
}

pub fn reload_env_file<P: AsRef<Path>>(file_path: P) -> Result<usize, Box<dyn std::error::Error>> {
    EnvManager::instance().reload_env_file(file_path)
}

pub fn print_env_status() {
    EnvManager::instance().print_status()
}

pub fn cleanup_env() {
    EnvManager::instance().cleanup()
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::fs;
//     use tempfile::tempdir;

//     #[test]
//     fn test_singleton_behavior() {
//         let manager1 = EnvManager::instance();
//         let manager2 = EnvManager::instance();
        
//         // Both should point to the same instance
//         assert!(std::ptr::eq(manager1, manager2));
//     }

//     #[test]
//     fn test_load_and_unload_env_file() {
//         let dir = tempdir().unwrap();
//         let env_file = dir.path().join(".env");
        
//         // Create test .env file
//         fs::write(&env_file, "TEST_VAR=test_value\nANOTHER_VAR=another_value").unwrap();
        
//         let manager = EnvManager::new(); // Use new() for testing to avoid global state
        
//         // Load the file
//         let loaded = manager.load_env_file(&env_file, None).unwrap();
//         assert_eq!(loaded, 2);
//         assert_eq!(env::var("TEST_VAR").unwrap(), "test_value");
//         assert_eq!(env::var("ANOTHER_VAR").unwrap(), "another_value");
        
//         // Unload the file
//         manager.unload_env_file(&env_file).unwrap();
//         assert!(env::var("TEST_VAR").is_err());
//         assert!(env::var("ANOTHER_VAR").is_err());
//     }

//     #[test]
//     fn test_multiple_files_with_override() {
//         let dir = tempdir().unwrap();
//         let env_file1 = dir.path().join(".env");
//         let env_file2 = dir.path().join(".env.test");
        
//         // Create test files
//         fs::write(&env_file1, "VAR1=from_env\nVAR2=from_env").unwrap();
//         fs::write(&env_file2, "VAR2=from_test\nVAR3=from_test").unwrap();
        
//         let manager = EnvManager::new();

//         // Load both files
//         let env_files: Vec<&std::path::PathBuf> = vec![&env_file1, &env_file2];
//         manager.load_env_variables_from_env_files(Some(&env_files), None);

//         assert_eq!(env::var("VAR1").unwrap(), "from_env");
//         assert_eq!(env::var("VAR2").unwrap(), "from_test"); // Should be overridden
//         assert_eq!(env::var("VAR3").unwrap(), "from_test");
        
//         // Remove first file
//         manager.unload_env_file(&env_file1).unwrap();
        
//         assert!(env::var("VAR1").is_err()); // Should be removed
//         assert_eq!(env::var("VAR2").unwrap(), "from_test"); // Should still exist
//         assert_eq!(env::var("VAR3").unwrap(), "from_test");
//     }

//     #[test]
//     fn test_convenience_functions() {
//         let dir = tempdir().unwrap();
//         let env_file = dir.path().join(".env");
        
//         fs::write(&env_file, "GLOBAL_TEST=global_value").unwrap();
        
//         // Test global convenience functions
//         let loaded = load_env_file(&env_file).unwrap();
//         assert_eq!(loaded, 1);
//         assert_eq!(env::var("GLOBAL_TEST").unwrap(), "global_value");
        
//         unload_env_file(&env_file).unwrap();
//         assert!(env::var("GLOBAL_TEST").is_err());
//     }
// }

// fn main() -> Result<(), Box<dyn std::error::Error>> {
//     println!("Environment Variables Manager (Singleton) Demo");
    
//     // Create some test .env files
//     fs::write(".env", "FOO=from_main_env\nBAR=main_value\nCOMMON=main_common")?;
//     fs::write(".env.test", "BAR=test_value\nBAZ=test_only\nCOMMON=test_common")?;
    
//     println!("1. Using global convenience functions...");
//     load_env_file(".env")?;
//     print_env_status();
    
//     println!("2. Loading .env.test file (will override some values)...");
//     load_env_file(".env.test")?;
//     print_env_status();
    
//     println!("3. Using singleton instance directly...");
//     let env_manager = EnvManager::instance();
    
//     println!("Current environment values:");
//     println!("  FOO = {:?}", env::var("FOO"));
//     println!("  BAR = {:?}", env::var("BAR"));
//     println!("  BAZ = {:?}", env::var("BAZ"));
//     println!("  COMMON = {:?}", env::var("COMMON"));
    
//     println!("\n4. Variable sources:");
//     println!("  FOO source: {:?}", env_manager.get_variable_source("FOO"));
//     println!("  BAR source: {:?}", env_manager.get_variable_source("BAR"));
//     println!("  BAZ source: {:?}", env_manager.get_variable_source("BAZ"));
    
//     println!("\n5. Simulating .env file change (removing BAR)...");
//     fs::write(".env", "FOO=updated_main_env\nCOMMON=updated_main_common")?;
//     reload_env_file(".env")?;
//     print_env_status();
    
//     println!("6. Environment values after .env reload:");
//     println!("  FOO = {:?}", env::var("FOO"));
//     println!("  BAR = {:?}", env::var("BAR")); // Should still exist from .env.test
//     println!("  BAZ = {:?}", env::var("BAZ"));
//     println!("  COMMON = {:?}", env::var("COMMON")); // Should be from .env.test
    
//     println!("\n7. Multiple instances point to same singleton:");
//     let manager1 = EnvManager::instance();
//     let manager2 = EnvManager::instance();
//     println!("  manager1 == manager2: {}", std::ptr::eq(manager1, manager2));
//     println!("  Files loaded: {}", manager1.loaded_files_count());
//     println!("  Variables managed: {}", manager1.managed_variables_count());
    
//     println!("\n8. Cleaning up using convenience function...");
//     cleanup_env();
//     print_env_status();
    
//     // Cleanup test files
//     let _ = fs::remove_file(".env");
//     let _ = fs::remove_file(".env.test");
    
//     println!("Demo completed!");
//     Ok(())
// }

// // Example usage for file watching scenario with singleton
// #[allow(dead_code)]
// fn file_watcher_example() -> Result<(), Box<dyn std::error::Error>> {
//     use std::time::Duration;
//     use std::thread;
    
//     // Initial load using convenience functions
//     load_env_variables_from_env_files(&[".env", ".env.local", ".env.development"])?;
    
//     // Simulate file watching loop
//     loop {
//         thread::sleep(Duration::from_secs(1));
        
//         // In a real implementation, you'd use a file watcher like `notify`
//         // For demo purposes, we'll just reload periodically
        
//         // Check if files were modified and reload them
//         for file_path in [".env", ".env.local", ".env.development"] {
//             if Path::new(file_path).exists() {
//                 // In real implementation, check modification time
//                 reload_env_file(file_path)?;
//             } else {
//                 // File was deleted, unload its variables
//                 unload_env_file(file_path)?;
//             }
//         }
        
//         break; // Exit for demo
//     }
    
//     Ok(())
// }