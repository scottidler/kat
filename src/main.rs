use clap::{Arg, ArgMatches, Command};
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::{Path, PathBuf}, process::Command as ShellCommand};

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    #[serde(skip)]
    name: String,
    about: String,
    included_paths: Vec<String>,
    excluded_paths: Vec<String>,
    included_types: Vec<String>,
    excluded_types: Vec<String>,
}

type Configs = HashMap<String, Config>;

#[derive(Debug)]
struct Kat {
    configs: Configs,
}

impl Kat {
    fn new(config_dir: PathBuf) -> Result<Self> {
        let configs = Kat::load_configs(&config_dir)?;
        Ok(Self { configs })
    }

    fn load_configs(config_dir: &Path) -> Result<Configs> {
        if !config_dir.exists() {
            return Err(eyre!("Config directory not found: {}", config_dir.display()));
        }

        let mut configs = Configs::new();

        for entry in fs::read_dir(config_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    if extension == "yml" || extension == "yaml" {
                        let config_content = fs::read_to_string(&path)?;
                        let mut config: Config = serde_yaml::from_str(&config_content)?;

                        if let Some(file_name) = path.file_stem() {
                            if let Some(name) = file_name.to_str() {
                                config.name = name.to_string();
                                configs.insert(name.to_string(), config);
                            }
                        }
                    }
                }
            }
        }

        Ok(configs)
    }

    fn config_to_command(config: &Config) -> Command {
        let command = Command::new(&config.name)
            .about(&config.about)
            .arg(Arg::new("path")
                .value_name("PATH")
                .default_value(".")
                .help("Path to start from (file or directory)")
                .required(false)
                )
            .arg(Arg::new("included-paths")
                .long("included-paths")
                .help("Included paths")
                .default_values(&config.included_paths))
            .arg(Arg::new("excluded-paths")
                .long("excluded-paths")
                .help("Excluded paths")
                .default_values(&config.excluded_paths))
            .arg(Arg::new("included-types")
                .long("included-types")
                .help("Included types")
                .default_values(&config.included_types))
            .arg(Arg::new("excluded-types")
                .long("excluded-types")
                .help("Excluded types")
                .default_values(&config.excluded_types));
        command
    }

    pub fn configs_to_command(configs: &Configs) -> Command {
        let mut command = Command::new("kat").about("Concatenate files with metadata")
            .arg(Arg::new("debug")
                 .short('d')
                 .long("debug")
                 .help("print only the paths, not the contents")
                 .action(clap::ArgAction::SetTrue));

        for config in configs.values() {
            let subcommand = Kat::config_to_command(config);
            command = command.subcommand(subcommand);
        }

        command
    }

    pub fn parse(configs: &Configs, args: &[String]) -> Result<ArgMatches> {
        let kat_command = Kat::configs_to_command(configs);
        match kat_command.try_get_matches_from(args) {
            Ok(matches) => Ok(matches),
            Err(err) if err.use_stderr() => Err(eyre!(err.to_string())),
            Err(err) => {
                err.print()?;
                std::process::exit(0);
            }
        }
    }

    pub fn run_subcommand(&self, subcommand: &str, path_override: Option<PathBuf>, debug: bool) -> Result<()> {
        let config = self.configs.get(subcommand).ok_or_else(|| eyre!("Config for '{}' not found", subcommand))?;

        let start_path = path_override.unwrap_or_else(|| PathBuf::from("."));

        if start_path.is_file() {
            if debug {
                println!("[DEBUG] Path: {}", start_path.display());
            } else {
                self.print_file_content(&start_path)?;
            }
            return Ok(());
        }

        let included_paths: Vec<PathBuf> = config
            .included_paths
            .iter()
            .map(|p| start_path.join(p))
            .collect();
        let excluded_paths: Vec<PathBuf> = config
            .excluded_paths
            .iter()
            .map(|p| start_path.join(p))
            .collect();

        for path in included_paths {
            if excluded_paths.contains(&path) {
                continue;
            }

            if path.is_file() {
                if debug {
                    println!("[DEBUG] Path: {}", path.display());
                } else {
                    self.print_file_content(&path)?;
                }
            } else if path.is_dir() {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    let file_path = entry.path();

                    if file_path.is_file() {
                        if debug {
                            println!("[DEBUG] Path: {}", file_path.display());
                        } else {
                            self.print_file_content(&file_path)?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn print_file_content(&self, path: &Path) -> Result<()> {
        println!("--- {} ---", path.display());

        let bat_available = ShellCommand::new("bat").output().is_ok();
        let viewer = if bat_available { "bat" } else { "cat" };

        let status = ShellCommand::new(viewer)
            .arg(path)
            .status()
            .map_err(|e| eyre!("Failed to run '{}': {}", viewer, e))?;

        if !status.success() {
            return Err(eyre!("{} command failed with status: {}", viewer, status));
        }

        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let config_dir = dirs::config_dir()
        .ok_or_else(|| eyre!("Failed to locate config directory"))?
        .join("kat");

    let kat = Kat::new(config_dir)?;

    let args: Vec<String> = std::env::args().collect();
    let matches = Kat::parse(&kat.configs, &args)?;
    let debug = matches.get_flag("debug");

    if let Some((subcommand, sub_matches)) = matches.subcommand() {
        let path_override = sub_matches.get_one::<String>("path").map(PathBuf::from);
        kat.run_subcommand(subcommand, path_override, debug)?;
    } else {
        println!("No subcommand provided.");
    }

    Ok(())
}
