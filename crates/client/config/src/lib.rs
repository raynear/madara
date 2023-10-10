use std::collections::HashMap;

use config::Config;
use config::File;
use config::Environment;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref DA_CONFIG: HashMap<String, String> = {
        let config = Config::builder()
        .add_source(File::with_name("../../Config.toml"))
        .add_source(Environment::with_prefix("da"))
        .build()
        .unwrap();
        
        let da_config = config.try_deserialize::<HashMap<String, String>>().unwrap();

        da_config
    };
}