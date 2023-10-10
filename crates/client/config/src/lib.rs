use std::collections::HashMap;

use config::Config;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref DA_CONFIG: HashMap<String, String> = {
        let config = Config::builder()
        // Add in `./Settings.toml`
        .add_source(config::File::with_name("./Config.toml"))
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
        .add_source(config::Environment::with_prefix("DA"))
        .build()
        .unwrap();
        
        let da_config = config.clone().try_deserialize::<HashMap<String, String>>().unwrap();

        da_config
    };
    // Print out our settings (as a HashMap)
}
// #[derive(Debug)]
// pub struct MyConfig {
// sequencer_private_key: String,
// da_host: String,
// da_namespace: String,
// da_auth_token: String,
// }
//
// Create a global instance of SyncDB that can be accessed from other modules.
// lazy_static! {
// pub static ref DA_CONFIG: MyConfig = {
// let mut settings = Config::default();
//
// Load configuration from a configuration file
// settings.merge(File::with_name("../../Config.toml"))?;
//
// Load environment variables
// settings.merge(Environment::new())?;
//
// Access the configuration values
// let my_config: MyConfig = serde_json::to_string(settings)?;
//
// println!("API Key: {}", my_config.da_host);
// println!("Port: {}", my_config.da_namespace);
// my_config
// };
// }
