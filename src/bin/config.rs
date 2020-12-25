use once_cell::sync::Lazy;
use serde_derive::Deserialize;

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config::new());

#[derive(Debug, Deserialize)]
pub struct Renderer {
    pub vertex_shader: String,
    pub fragment_shader: String,
    pub css: String,
    pub max_tiles: usize,
    pub max_features: u64,
    pub tile_size: u32,
    pub msaa_samples: u32,
    pub selection_tags: Vec<String>,
    pub ui_font: String,
    pub temperature: Temperature,
}

impl Default for Renderer {
    fn default() -> Self {
        Self {
            vertex_shader: "config/shader.vert".to_string(),
            fragment_shader: "config/shader.frag".to_string(),
            css: "config/style.css".to_string(),
            max_tiles: 200,
            max_features: 1000,
            tile_size: 384,
            msaa_samples: 4,
            selection_tags: Default::default(),
            ui_font: "config/Ruda-Bold.ttf".to_string(),
            temperature: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Temperature {
    pub vertex_shader: String,
    pub fragment_shader: String,
}

impl Default for Temperature {
    fn default() -> Self {
        Self {
            vertex_shader: "config/temperature/shader.vert".to_string(),
            fragment_shader: "config/temperature/shader.frag".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct General {
    pub log_level: log::Level,
    pub display_framerate: bool,
    pub data_root: String,
}
impl Default for General {
    fn default() -> Self {
        Self {
            log_level: log::Level::Warn,
            display_framerate: false,
            data_root: Default::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub general: General,
    pub renderer: Renderer,
}

impl Config {
    pub fn new() -> Self {
        // Add in a local configuration file
        // This file shouldn't be checked in to git
        let config = std::path::Path::new("config/local");
        if std::path::Path::exists(config) {
            toml::from_str(&std::fs::read_to_string(config).unwrap()).unwrap()
        } else {
            Default::default()
        }
    }
}
