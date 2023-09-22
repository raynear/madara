use std::path::Path;
use std::time::Duration;
use std::{env, str, thread, time};

use base64::engine::general_purpose;
use base64::Engine as _;
use bincode::{deserialize, serialize};
use dotenv::dotenv;
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use hyper::{Body, Client, Request};
use lazy_static::lazy_static;
use rocksdb::{Error, IteratorMode, DB};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tokio;
use tokio::runtime::Runtime;

// Import Lazy from the lazy_static crate
// Import the Error type from rocksdb crate
// Define a struct to hold the DB instance.
pub struct MyDatabase {
    db: DB,
}

impl MyDatabase {
    // Constructor to open the database.
    fn open() -> Result<MyDatabase, Error> {
        let path = Path::new("epool");
        let db = DB::open_default(&path)?;
        Ok(MyDatabase { db })
    }

    // Method to perform a read operation.
    fn read<K, V>(&self, key: K) -> V
    where
        K: Serialize,
        V: DeserializeOwned, // Ensure T can be deserialized.
    {
        // Serialize key to bytes
        let key_bytes = serialize(&key).unwrap();

        // Use the 'get' method to retrieve the value.
        let result_of_get = self.db.get(key_bytes);

        let option = match result_of_get {
            Ok(val) => val,
            Err(err) => {
                eprintln!("Failed to read from DB: {:?}", err);
                None
            }
        };

        // Handle the None case.
        let my_vec = match option {
            Some(val) => val,
            None => Vec::new(),
        };
        println!("RIGHT BEFORE DESER ERROR: my_vec: {:?}", my_vec);
        // Deserialize the data into the desired type T.
        let mut retries = 0;
        let max_retries = 5;
        let mut value: Option<V> = None;

        while retries < max_retries {
            match deserialize(&my_vec) {
                Ok(val) => {
                    value = Some(val);
                    // Break the loop on successful deserialization
                    break;
                }
                Err(err) => {
                    retries += 1;
                    eprintln!(
                        "Failed to deserialize, the error is: {:?}, number of trials: {} of {}",
                        err, retries, max_retries
                    )
                }
            }
        }
        // If value is still None after retries, you can handle it as needed.
        let value = value.unwrap_or_else(|| {
            // Handle the case where deserialization still failed after max retries.
            // You can return a default value or panic, depending on your use case.
            panic!("Failed to deserialize after {} retries", max_retries);
        });

        value
    }

    // Method to perform a write operation.
    pub fn write<K, V>(&self, key: K, value: V)
    where
        K: Serialize,
        V: Serialize,
    {
        // Serialize key to bytes
        let key_bytes = serialize(&key).unwrap();
        // Serialize value for storing in DB
        let value_bytes = serialize(&value).unwrap();
        let result_of_put = self.db.put(key_bytes, value_bytes);
        match result_of_put {
            Ok(()) => {
                self.display_all();
            }
            Err(err) => eprintln!("Failed to write to DB: {:?}", err),
        };
    }

    fn clear_db(&self) {
        // Create an iterator starting at the first key.
        let iter = self.db.iterator(IteratorMode::Start);

        // Iterate through all key-value pairs and print them.
        for result in iter {
            let deleted = self.db.delete(result.unwrap().0);
            match deleted {
                Ok(()) => {}
                Err(err) => {
                    eprintln!("Failed to delete, error: {:?}", err);
                }
            }
        }
    }

    fn display_all(&self) {
        // Create an iterator starting at the first key.
        let iter = self.db.iterator(IteratorMode::Start);

        // Iterate through all key-value pairs and print them.
        for result in iter {
            match result {
                Ok((key, value)) => {
                    // println!("display_all_data: key: {:?} value {:?}", key, value);
                    println!("display_all_data: key: {:?} value.len(): {:?}", key, value.len());
                }
                Err(err) => {
                    eprintln!("There is an error! {:?}", err);
                }
            }
        }
    }

    fn get_next_entry<K>(&self, start_key: K) -> K
    where
        K: Serialize + DeserializeOwned,
    {
        // Serialize key to bytes. The process is 2-step since u64 does not directly support as_ref()
        let serialized = serialize(&start_key).unwrap();
        let key_bytes = serialized.as_ref();

        // Create an iterator starting from the key after the specified start_key.
        let mut iter = self.db.iterator(IteratorMode::From(key_bytes, rocksdb::Direction::Forward));

        // Iterate to get the next entry.
        // Note: I am bravely unwrapping it because we would not iterate once we reach the last entry, since
        // higher-level condition would not call this function in such case
        let key = iter.next().unwrap().unwrap().0;
        let key_vec = key.into_vec();
        return deserialize(&key_vec).unwrap();
    }
}

// Create a global instance of MyDatabase that can be accessed from other modules.
lazy_static! {
    pub static ref SYNC_DB: MyDatabase = MyDatabase::open().unwrap_or_else(|err| {
        eprintln!("Failed to open database: {:?}", err);
        std::process::exit(1); // Exit the program on error
    });
}

fn convert_vec_to_string(txs: Vec<u8>) -> String {
    // Check if the conversion was successful
    let txs_string = match String::from_utf8(txs) {
        Ok(val) => {
            println!("Converted to str");
            val
        }
        Err(err) => {
            eprintln!("Conversion to str failed: {:?}", err);
            "".to_string()
        }
    };
    txs_string
}

fn encode_data_to_base64(original: String) -> String {
    // Convert string to bytes
    let bytes = original.as_bytes();
    // Convert bytes to base64
    let base64_str: String = general_purpose::STANDARD.encode(&bytes);
    base64_str
}

async fn submit_to_da(data: Vec<u8>) -> String {
    dotenv().ok();
    let da_host = env::var("DA_HOST").expect("DA_HOST must be set");
    let da_namespace = env::var("DA_NAMESPACE").expect("DA_NAMESPACE must be set");
    let da_auth_token = env::var("DA_AUTH_TOKEN").expect("DA_AUTH_TOKEN must be set");
    let da_auth = format!("Bearer {}", da_auth_token);

    let data_string = convert_vec_to_string(data);

    let encoded_data = encode_data_to_base64(data_string);

    let client = Client::new();
    let rpc_request = json!({
        "jsonrpc": "2.0",
        "method": "blob.Submit",
        "params": [
            [
                {
                    "namespace": da_namespace,
                    "data": encoded_data,
                }
            ]
        ],
        "id": 1,
    });

    let uri = std::env::var("da_uri").unwrap_or(da_host.into());

    let req = Request::post(uri.as_str())
        .header(AUTHORIZATION, HeaderValue::from_str(da_auth.as_str()).unwrap())
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(rpc_request.to_string()))
        .unwrap();
    let response_future = client.request(req);

    let resp = tokio::time::timeout(Duration::from_secs(100), response_future)
        .await
        .map_err(|_| "Request timed out")
        .unwrap()
        .unwrap();

    let response_body = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let parsed: Value = serde_json::from_slice(&response_body).unwrap();

    // println!("stompesi - {:?}", parsed);

    if let Some(result_value) = parsed.get("result") { result_value.to_string() } else { "".to_string() }
}

pub fn sync_with_da() {
    println!(
        "
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@&&####&&@@@@@@@#&@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@&P?777J5B&@@@@@#G5?!~^^::::::^^~!?5BJ!@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@B: :7?7~^:^!J5?~::~7J5PGB####BGP5J7~  :75#@@@@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@7  G@@@@@B7  .  7B@@@@@@@@@@@@@@@@@Y.55!:.~Y#@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@J  J@@@B7..7G&#57~!Y#@@@@@@@@@@@@@Y Y@@@&G7..7B@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@#:  5#7 :Y#@@@@@@&G?!7P&@@@@@@@@@Y ?@@@@@@@&Y: 7B@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@G. ...J&@@@@@@@@@@@&GJ!JB@@#PJJ7 ~@@@@@@@@@@&Y..Y@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@5   !&@@@@@@@@@@@@@@@&P??7.     .?&@@@@@@@@@@#~ !&@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@&! .  ^G@@@@@@@@@@@@@&&@@J         Y@@@@@@@@@@@@7 ~&@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@7 !G:   ?P5YYJJJJJYYYJJJY?         !YJYYY55PGBB#&7 !@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@&BP?  ^!~~    !GBB###&&&&&@@@@G:     .P&##BBGP5YJ?7!!~  ?PB&@@@@@@@@@@@@@
        @@@@@@@@@&P?^::^  ?##&@5.   ~G@@@@@@@@@@@@@5. ?GBBJ?G@@@@@@@@@@@&&#J  ~^:~?P&@@@@@@@@@
        @@@@@@@&J: ~JG&B..B@@@@@#!    !G@@@@@@@@@@Y  !@@@@@#J7G@@@@@@@@@@@@#. B&BY! :J&@@@@@@@
        @@@@@@@7 .G@@@@P .#@@@@@@@5:    7B@@@@@@@Y  ^&@@@@@@@B?7G@@@@@@@@@@&: P@@@@B: 7@@@@@@@
        @@@@@@@J  ?G&@@G .#@@@@@@@@&?.   .7B@@@@Y  :#@@@@@@@@@@G7?#@@@@@@@@&: G@@&B?  J@@@@@@@
        @@@@@@@@P~  :!JY. G@@@@@@@@@@#7.   .7B@Y  .G@@@@@@@@@@@@@P!J&@@@@@@G  YY!:  ~P@@@@@@@@
        @@@@@@@@@@BY!:    .^~7?JY5PGGBB5~    .~   Y@&&&&&&&####BBBP^^YYJ7!~:    :!YB@@@@@@@@@@
        @@@@@@@@@@@@@&BPJ. ..          ..         :::::::::....          .. .JPB&@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@Y :GBP5J?!~^:...                    ..:^^~7?J7 ?G: J@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@J :B@@@@@@@&&##BB7   .     :?PGGGBB##&&@@@@@@Y.: J@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@5. Y@@@@@@@@@@@5   :BG7:   .!5&@@@@@@@@@@@@@5  :#@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@B~ ~G@@@@@@@@5   .G@@@#5!.   :7G&@@@@@@@@G~ !Y ^#@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@P^ ~5&@@@@Y    5@@@@@@@#Y~.   ^JG&@@&P~ ^5@@J ~&@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@&P!.:7P&5    J@@@@@@@@@@@#5!:   ^??:.!G@@@@&: 5@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@#Y~::    !@@@@@@@@@@@@@@&B?.     :JPB#&&G. Y@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@@J     :~!7JJYYYYJJ7!^::^!JGGY7^.  .:: .?&@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@P    :BG5Y?77!~~!77?Y5G#&@@@@@@&#BGP55G#@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@G^..^B@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@@&##&@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
        @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
    "
    );

    SYNC_DB.clear_db();

    let three_seconds = time::Duration::from_millis(3000);

    let mut txs: Vec<u8>;
    let mut sync_target: u64 = 0;
    let mut sync: u64 = 0;
    let mut block_height: String = "".to_string();

    SYNC_DB.write("sync", sync);
    SYNC_DB.write("sync_target", sync_target);

    // Create the runtime
    let rt = match Runtime::new() {
        Ok(rt) => {
            println!("Successfully created runtime");
            rt
        }
        Err(err) => {
            eprintln!("Error in creating runtime: {}", err);
            return;
        }
    };

    loop {
        thread::sleep(three_seconds);
        sync_target = SYNC_DB.read("sync_target");
        sync = SYNC_DB.read("sync");

        println!("sync_target: {:?} and sync {:?}", sync_target, sync);
        if sync_target != sync {
            SYNC_DB.display_all();
            let next_entry = SYNC_DB.get_next_entry(sync);
            txs = SYNC_DB.read(next_entry);
            rt.block_on(async {
                block_height = submit_to_da(txs).await;
                println!("DA BLOCK HEIGHT-------------->: {}", block_height);
            });
            if block_height.len() != 0 {
                SYNC_DB.write("sync", next_entry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = 4; //= sumit_to(2, 2);
        assert_eq!(result, 4);
    }
}
