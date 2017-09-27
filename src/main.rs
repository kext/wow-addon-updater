// WoW Addon Updater by Lukas Joeressen
// Licensed under CC0 - https://creativecommons.org/CC0

use std::io::{Read, Seek};
use std::path::{Path, PathBuf, Component};
use std::fmt::Display;
extern crate reqwest;
extern crate regex;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate zip;
use zip::ZipArchive;
extern crate s_app_dir;

trait StringError<T> {
  fn stringerror(self) -> Result<T, String>;
}

impl<T, E: Display> StringError<T> for Result<T, E> {
  fn stringerror(self) -> Result<T, String> {
    self.map_err(|e| format!("{}", e))
  }
}

/// Download a url into a byte buffer.
fn get_data(url: &str, client: &reqwest::Client) -> Result<Vec<u8>, String> {
  let mut resp = client.get(url).stringerror()?
    .send().stringerror()?;
  if !resp.status().is_success() {
    return Err(format!("Error {}", resp.status()));
  }
  let mut data: Vec<u8> = Vec::new();
  resp.read_to_end(&mut data).stringerror()?;
  Ok(data)
}

/// Download a url into a String.
fn get_string(url: &str, client: &reqwest::Client) -> Result<String, String> {
  let data = get_data(url, client)?;
  let content = String::from_utf8_lossy(&data);
  Ok(content.into_owned())
}

/// Get the version of an addon hosted on Curse.
fn get_curse_version(url: &str, client: &reqwest::Client) -> Result<String, String> {
  let data = get_string(url, client)?;
  let re = regex::Regex::new("<li class=\"newest-file\">Newest File: ([^<]+)</li>").unwrap();
  let version = re.captures(&data);
  let re = regex::Regex::new("data-epoch=\"([0-9]+)\"").unwrap();
  let date = re.captures(&data);
  if version.is_none() || date.is_none() {
    return Err(String::from("Could not get version."));
  }
  Ok(format!("{} ({})", version.unwrap()[1].to_string(), date.unwrap()[1].to_string()))
}

/// Download an addon hosted on Curse.
fn get_curse_download(url: &str, client: &reqwest::Client) -> Result<Vec<u8>, String> {
  let url = format!("{}/download", url);
  let data = get_string(&url, client)?;
  let re = regex::Regex::new("data-href=\"([^\"]+)\"").unwrap();
  let url = match re.captures(&data) {
    Some(url) => url[1].to_string(),
    None => {
      return Err(String::from("Could not get download link."));
    },
  };
  get_data(&url, client)
}

fn sanitize_path(path: &str) -> PathBuf {
  let path = Path::new(path);
  let mut res = PathBuf::new();
  for c in path.components() {
    match c {
      Component::Normal(c) => {
        res.push(c);
      },
      Component::ParentDir => {
        res.pop();
      },
      _ => {},
    }
  }
  res
}

/// Unpack an addon to a folder.
fn install_addon<R: Read + Seek>(zip: &mut R, folder: &str, owned_folders: &Vec<String>) -> Result<Vec<String>, String> {
  let folder = Path::new(folder);
  let mut folders: Vec<String> = Vec::new();
  let mut archive = ZipArchive::new(zip).stringerror()?;
  for i in 0..archive.len() {
    let file = archive.by_index(i).unwrap();
    if file.name().ends_with("/") {
      continue;
    }
    let path = sanitize_path(file.name());
    if path.components().next().is_none() {
      continue;
    }
    let f = String::from(path.components().next().unwrap().as_ref().to_str().unwrap());
    if !folders.contains(&f) {
      folders.push(f);
    }
  }
  for s in &folders {
    let exists = folder.join(s).exists();
    if exists && !owned_folders.contains(s) {
      return Err(format!("{} already exists", s));
    }
  }
  for s in owned_folders {
    let dir = folder.join(s);
    if dir.is_dir() {
      std::fs::remove_dir_all(dir).stringerror()?;
    } else {
      std::fs::remove_file(dir).stringerror()?;
    }
  }
  for i in 0..archive.len() {
    let mut file = archive.by_index(i).unwrap();
    if file.name().ends_with("/") {
      continue;
    }
    let path = sanitize_path(file.name());
    if path.components().next().is_none() {
      continue;
    }
    {
      let parent = path.parent().unwrap().to_str().unwrap();
      if parent != "" {
        std::fs::create_dir_all(&folder.join(parent)).stringerror()?;
      }
    }
    let mut outfile = std::fs::File::create(&folder.join(path)).stringerror()?;
    std::io::copy(&mut file, &mut outfile).stringerror()?;
  }
  Ok(folders)
}

#[derive(Serialize, Deserialize)]
struct Addons {
  addon_folder: String,
  addons: Vec<Addon>,
}

impl Addons {
  fn is_installed(&self, url: &str) -> bool {
    for addon in &self.addons {
      if url == &addon.url {
        return true;
      }
    }
    return false;
  }

  fn check_folder(&self) {
    if &self.addon_folder == "" {
      println!("\x1b[31;1mYou have to set your addons folder.\x1b[0m");
      std::process::exit(1);
    } else if !Path::new(&self.addon_folder).is_dir() {
      println!("\x1b[31;1mYour addons folder is not a directory.\x1b[0m");
      std::process::exit(1);
    }
  }
}

#[derive(Serialize, Deserialize)]
struct Addon {
  url: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  installed: Option<String>,
  #[serde(default)]
  folders: Vec<String>,
}

impl Addon {
  fn new(url: &str) -> Addon {
    Addon{
      url: String::from(url),
      installed: None,
      folders: Vec::new(),
    }
  }
}

fn config_path() -> PathBuf {
  let app_dir = s_app_dir::AppDir::new("addons");
  let cfg = app_dir.xdg_dir(s_app_dir::XdgDir::Config).unwrap();
  cfg.join("addons.json")
}

fn discover_addon_folder() -> String {
  if cfg!(target_os = "macos") {
    let path = String::from("/Applications/World of Warcraft/Interface/Addons");
    if Path::new(&path).is_dir() {
      return path;
    }
  } else {
    let path = String::from("C:\\Program Files (x86)\\World of Warcraft\\Interface\\Addons");
    if Path::new(&path).is_dir() {
      return path;
    }
  }
  String::from("")
}

fn save_addons(addons: &Addons) {
  let cfg = config_path();
  std::fs::create_dir_all(Path::new(&cfg).parent().unwrap()).unwrap();
  let mut file = std::fs::File::create(&cfg).unwrap();
  serde_json::to_writer_pretty(&mut file, addons).unwrap();
}

fn addons_default() -> Addons {
  let addons = Addons {
    addon_folder: discover_addon_folder(),
    addons: Vec::new(),
  };
  save_addons(&addons);
  addons
}

fn load_addons() -> Addons {
  match std::fs::File::open(&config_path()) {
    Ok(mut file) => match serde_json::from_reader(&mut file) {
      Ok(addons) => addons,
      Err(_) => addons_default(),
    },
    Err(_) => addons_default(),
  }
}

fn update_addons(addons: &mut Addons) {
  let client = reqwest::Client::new().unwrap();
  for addon in &mut addons.addons {
    match get_curse_version(&addon.url, &client) {
      Ok(version) => {
        if addon.installed.is_none() || addon.installed.as_ref().unwrap() != &version {
          println!("\x1b[32;1mUpdating\x1b[0m {}", &addon.url);
          match get_curse_download(&addon.url, &client) {
            Ok(data) => {
              let mut data = std::io::Cursor::new(&data);
              match install_addon(&mut data, &addons.addon_folder, &addon.folders) {
                Ok(folders) => {
                  println!("-> {}", version);
                  addon.folders = folders;
                  addon.installed = Some(version);
                },
                Err(error) => {
                  println!("\x1b[31;1m{}\x1b[0m", error);
                },
              }
            },
            Err(error) => {
              println!("\x1b[31;1m{}\x1b[0m", error);
            },
          }
        }
      },
      Err(error) => println!("\x1b[31;1m{}\x1b[0m", error),
    }
  }
}

fn install_new(addon: &mut Addon, addon_folder: &str, client: &reqwest::Client) -> bool {
  match get_curse_version(&addon.url, client) {
    Ok(version) => {
      println!("\x1b[32;1mInstalling\x1b[0m {}", &addon.url);
      match get_curse_download(&addon.url, client) {
        Ok(data) => {
          let mut data = std::io::Cursor::new(&data);
          match install_addon(&mut data, addon_folder, &addon.folders) {
            Ok(folders) => {
              println!("-> {}", version);
              addon.folders = folders;
              addon.installed = Some(version);
              return true;
            },
            Err(error) => {
              println!("\x1b[31;1m{}\x1b[0m", error);
              return false;
            },
          }
        },
        Err(error) => {
          println!("\x1b[31;1m{}\x1b[0m", error);
          return false;
        },
      }
    },
    Err(error) => {
      println!("\x1b[31;1m{}\x1b[0m", error);
      return false;
    },
  }
}

fn help() {
  println!("install [url...]   Install new addons.");
  println!("update             Update all addons.");
  println!("help               Show this help.");
  println!("");
  println!("Your configuration is here:");
  println!("{}", config_path().to_string_lossy());
}

fn main() {
  let args: Vec<String> = std::env::args().collect();
  if args.len() < 2 {
    help();
  } else if args[1] == "install" {
    let mut addons = load_addons();
    addons.check_folder();
    let client = reqwest::Client::new().unwrap();
    for url in &args[2..] {
      let mut addon = Addon::new(url);
      if addons.is_installed(url) {
        println!("{} is already installed.", url);
      } else if install_new(&mut addon, &addons.addon_folder, &client) {
        addons.addons.push(addon);
      }
    }
    save_addons(&addons);
  } else if args[1] == "update" {
    let mut addons = load_addons();
    addons.check_folder();
    update_addons(&mut addons);
    save_addons(&addons);
  } else {
    help();
  }
}
