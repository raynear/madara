use std::collections::HashMap;

use config::Config;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref DA_CONFIG: HashMap<String, String> = {
        println!("{:?}",std::env::current_dir());
        let config = Config::builder()
        // Add in `./Settings.toml`
        .add_source(config::File::with_name("/Users/abcdefgh/Desktop/radius/madara/Config.toml"))
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
        .add_source(config::Environment::with_prefix("DA"))
        .build()
        .unwrap();
        
        let da_config = config.try_deserialize::<HashMap<String, String>>().unwrap();

        da_config
    };
    // Print out our settings (as a HashMap)
}