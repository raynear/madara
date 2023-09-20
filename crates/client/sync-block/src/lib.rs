// let a: Vec<_> = txs.encrypted_pool.values().cloned().collect();
// println!("self.txs.get_mut: {:?}", a);

use std::collections::HashMap;
use std::path::Path;
use std::str::Bytes;
use std::time::Duration;
use std::{env, str, thread, time};

use base64::engine::general_purpose;
use base64::Engine as _;
use bincode::{deserialize, serialize};
use dotenv::dotenv;
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use hyper::{Body, Client, Request};
use rocksdb::{Error, ErrorKind, IteratorMode, DB};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio;
use tokio::runtime::Runtime; // Import the Error type from rocksdb crate

// Define a struct to hold the DB instance.
pub struct MyDatabase {
    db: DB,
}

impl MyDatabase {
    // Constructor to open the database.
    fn open() -> Result<MyDatabase, Error> {
        let path = Path::new("epool");
        let db = DB::open_default(&path)?;
        // let db = DB::open_default(&path).expect("Failed to open database");
        Ok(MyDatabase { db })
    }

    // Method to perform a read operation.
    fn read<K, V>(&self, key: K) -> V
    where
        K: AsRef<[u8]>,
        V: DeserializeOwned, // Ensure T can be deserialized.
    {
        // Serialize key to bytes
        let key_bytes = key.as_ref();

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

        // Deserialize the data into the desired type T.
        let value: V = deserialize(&my_vec).unwrap();

        value
    }

    // Method to perform a write operation.
    fn write<K, V>(&self, key: K, value: V)
    where
        K: AsRef<[u8]>,
        V: Serialize,
    {
        // Serialize key to bytes
        let key_bytes = key.as_ref();
        // Serialize value for storing in DB
        let value_bytes = serialize(&value).unwrap();
        let result_of_put = self.db.put(key_bytes, value_bytes);
        match result_of_put {
            Ok(()) => println!("Successfully wrote to DB!"),
            Err(err) => eprintln!("Failed to write to DB: {:?}", err),
        };
    }

    fn clear_db(&self) {
        // Create an iterator starting at the first key.
        let iter = self.db.iterator(IteratorMode::Start);

        // Iterate through all key-value pairs and print them.
        for result in iter {
            match result {
                Ok((key, value)) => {
                    // println!("display_all_data: key: {:?} value {:?}", key, value);
                    let deleted = self.db.delete(key);
                    match deleted {
                        Ok(()) => {
                            println!("Deleted successfully")
                        }
                        Err(err) => {
                            eprintln!("There is an error! {:?}", err);
                        }
                    }
                }
                Err(err) => {
                    eprintln!("There is an error! {:?}", err);
                }
            }
        }
    }

    fn display_db(&self) {
        // Create an iterator starting at the first key.
        let iter = self.db.iterator(IteratorMode::Start);

        // Iterate through all key-value pairs and print them.
        for result in iter {
            match result {
                Ok((key, value)) => {
                    // println!("display_all_data: key: {:?} value {:?}", key, value);
                    println!("display_all_data: key: {:?}", key);
                }
                Err(err) => {
                    eprintln!("There is an error! {:?}", err);
                }
            }
        }
    }

    fn get_next_entry<K, V>(&self, start_key: K) -> (K, V)
    where
        K: Serialize + DeserializeOwned,
        V: DeserializeOwned,
    {
        // Serialize key to bytes. The process is 2-step since u64 does not directly support as_ref()
        let key_bytes = serialize(&start_key).unwrap().as_ref();

        // Create an iterator starting from the key after the specified start_key.
        let mut iter = self.db.iterator(IteratorMode::From(key_bytes, rocksdb::Direction::Forward));

        // Iterate to get the next entry.
        // Note: I am bravely unwrapping it because we would not iterate once we reach the last entry, since
        // higher-level condition would not call this function in such case
        let (key, value) = iter.next().unwrap().unwrap();
        let (key_vec, value_vec) = (key.into_vec(), value.into_vec());
        return (deserialize(&key_vec).unwrap(), deserialize(&value_vec).unwrap());
    }
}

fn encode_data_to_base64(original: &str) -> String {
    // Convert string to bytes
    let bytes = original.as_bytes();
    // Convert bytes to base64
    let base64_str: String = general_purpose::STANDARD.encode(&bytes);
    base64_str
}

async fn submit_to_da(data: &str) -> String {
    dotenv().ok();
    let da_host = env::var("DA_HOST").expect("DA_HOST must be set");
    let da_namespace = env::var("DA_NAMESPACE").expect("DA_NAMESPACE must be set");
    let da_auth_token = env::var("DA_AUTH_TOKEN").expect("DA_AUTH_TOKEN must be set");
    let da_auth = format!("Bearer {}", da_auth_token);

    let client = Client::new();
    let rpc_request = json!({
        "jsonrpc": "2.0",
        "method": "blob.Submit",
        "params": [
            [
                {
                    "namespace": da_namespace,
                    "data": data,
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
    println!("**********sync_with_da STARTED!**********");

    // Open or create a RocksDB database.
    let my_db = match MyDatabase::open() {
        Ok(db) => {
            println!("Successfully opened a DB instance");
            db
        }
        Err(err) => {
            eprintln!("Failed to open database: {:?}", err);
            return;
        }
    };

    my_db.write("sync", "Hello World");

    my_db.clear_db();
    println!("Clearance");

    my_db.display_db();
    println!("First display after writing");

    let three_seconds = time::Duration::from_millis(3000);

    let mut txs: Vec<u8>;
    let mut sync_target: u64 = 0;
    let mut sync: u64 = 0;
    let mut block_height: String = "".to_string();

    my_db.write("sync", sync);
    my_db.write("sync_target", sync_target);

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
        println!("SLEEPING FOR 3 SECONDS");
        thread::sleep(three_seconds);
        sync_target = my_db.read("sync_target");
        sync = my_db.read("sync");

        println!("sync_target: {:?} and sync {:?}", sync_target, sync);
        if sync_target != sync {
            my_db.display_db();

            println!("this is sync_bin inside if: {:?}", sync);
            println!("this is sync_target inside if: {:?}", sync_target);

            let (key, value) = my_db.get_next_entry(sync);
            txs = my_db.read(key);
            println!("these are the txs: {:?}", txs.len());
            let s = match str::from_utf8(&txs) {
                Ok(v) => v,
                Err(e) => panic!("Invalid UTF-8 sequence: {}", e),
            };
            rt.block_on(async {
                block_height = submit_to_da(&encode_data_to_base64(s)).await;
                println!("this is the block height from DA: {}", block_height);
            });
            if !(block_height.len() == 0) {
                my_db.write("sync", key);
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
