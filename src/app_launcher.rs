use std::{
    collections::HashSet,
    fs,
    process::Command,
    path::PathBuf,
};
use xdg::BaseDirectories;
use rayon::prelude::*;
use crate::cache::{update_cache, RECENT_APPS_CACHE};
use crate::gui::AppInterface;
use crate::config::{Config, load_config, get_current_time_in_timezone};

fn get_desktop_entries() -> Vec<PathBuf> {
    let xdg_dirs = BaseDirectories::new().unwrap();
    let data_dirs = xdg_dirs.get_data_dirs();

    data_dirs.par_iter()
        .flat_map(|dir| {
            let desktop_files = dir.join("applications");
            fs::read_dir(&desktop_files).ok()
                .into_iter()
                .flat_map(|entries| entries.filter_map(Result::ok))
                .map(|entry| entry.path())
                .filter(|path| path.extension().map_or(false, |ext| ext == "desktop"))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn parse_desktop_entry(path: &PathBuf) -> Option<(String, String)> {
    let content = fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut exec = None;
    for line in content.lines() {
        if line.starts_with("Name=") {
            name = Some(line[5..].trim().to_string());
        } else if line.starts_with("Exec=") {
            exec = Some(line[5..].trim().to_string());
        }
        if name.is_some() && exec.is_some() {
            break;
        }
    }
    name.zip(exec).map(|(name, exec)| {
        let placeholders = ["%f", "%u", "%U", "%F", "%i", "%c", "%k"];
        let cleaned_exec = placeholders.iter().fold(exec, |acc, &placeholder| 
            acc.replace(placeholder, "")
        ).trim().to_string();
        (name, cleaned_exec)
    })
}

fn search_applications(query: &str, applications: &[(String, String)], max_results: usize) -> Vec<(String, String)> {
    let query = query.to_lowercase();
    let mut unique_results = HashSet::new();
    
    applications.iter()
        .filter(|(name, _)| name.to_lowercase().contains(&query))
        .filter_map(|(name, exec)| {
            if unique_results.insert(name.clone()) {
                Some((name.clone(), exec.clone()))
            } else {
                None
            }
        })
        .take(max_results)
        .collect()
}

fn launch_app(app_name: &str, exec_cmd: &str, enable_recent_apps: bool) -> Result<(), Box<dyn std::error::Error>> {
    update_cache(app_name, enable_recent_apps)?;

    let home_dir = dirs::home_dir().ok_or("Failed to find home directory")?;
    Command::new("sh")
        .arg("-c")
        .arg(exec_cmd)
        .current_dir(home_dir)
        .spawn()?;
    Ok(())
}

pub struct AppLauncher {
    query: String,
    applications: Vec<(String, String)>,
    search_results: Vec<(String, String)>,
    is_quit: bool,
    config: Config,
}

impl Default for AppLauncher {
    fn default() -> Self {
        let config = load_config();
        let applications: Vec<(String, String)> = get_desktop_entries()
            .par_iter()
            .filter_map(|path| parse_desktop_entry(path))
            .collect();

        let search_results = if config.enable_recent_apps {
            let recent_apps_cache = RECENT_APPS_CACHE.lock().expect("Failed to acquire read lock");
            recent_apps_cache.recent_apps.iter()
                .filter_map(|app_name| {
                    applications.iter()
                        .find(|(name, _)| name == app_name)
                        .cloned()
                })
                .take(config.max_search_results)
                .collect()
        } else {
            Vec::new()
        };

        Self {
            query: String::new(),
            search_results,
            applications,
            is_quit: false,
            config,
        }
    }
}

impl AppInterface for AppLauncher {
    fn update(&mut self) {
        if self.is_quit {
            std::process::exit(0);
        }
    }

    fn handle_input(&mut self, input: &str) {
        match input {
            "ESC" => self.is_quit = true,
            "ENTER" => self.launch_first_result(),
            "P" if self.config.enable_power_options => crate::power::power_off(),
            "R" if self.config.enable_power_options => crate::power::restart(),
            "L" if self.config.enable_power_options => crate::power::logout(),
            _ => {
                self.query = input.to_string();
                self.search_results = search_applications(&self.query, &self.applications, self.config.max_search_results);
            }
        }
    }

    fn should_quit(&self) -> bool {
        self.is_quit
    }

    fn get_query(&self) -> String {
        self.query.clone()
    }

    fn get_search_results(&self) -> Vec<String> {
        self.search_results.iter().map(|(name, _)| name.clone()).collect()
    }

    fn get_time(&self) -> String {
        get_current_time_in_timezone(&self.config)
    }

    fn launch_app(&mut self, app_name: &str) {
        if let Some((_, exec_cmd)) = self.search_results.iter().find(|(name, _)| name == app_name) {
            if let Err(err) = launch_app(app_name, exec_cmd, self.config.enable_recent_apps) {
                eprintln!("Failed to launch app: {}", err);
            } else {
                self.is_quit = true;
            }
        }
    }

    fn get_config(&self) -> &Config {
        &self.config
    }
}

impl AppLauncher {
    fn launch_first_result(&mut self) {
        if let Some((app_name, exec_cmd)) = self.search_results.first() {
            if let Err(err) = launch_app(app_name, exec_cmd, self.config.enable_recent_apps) {
                eprintln!("Failed to launch app: {}", err);
            } else {
                self.is_quit = true;
            }
        }
    }
}