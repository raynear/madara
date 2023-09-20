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
    fn read<T>(&self, key: &[u8]) -> Result<T, Error>
    where
        T: DeserializeOwned, // Ensure T can be deserialized.
    {
        // Use the 'get' method to retrieve the value.
        let value = self.db.get(key)?;

        // Deserialize the data into the desired type T.

        // Handle the None case.
        let my_vec = match value {
            Some(val) => val,
            None => Vec::new(),
        };

        // Deserialize the data into the desired type T.
        let result: T = deserialize(&my_vec).unwrap();

        Ok(result)
    }

    // Method to perform a write operation.
    fn write<K, V>(&self, key: K, value: V) -> Result<(), Error>
    where
        K: AsRef<[u8]>,
        V: Serialize,
    {
        let key_bytes = key.as_ref();
        let value_bytes = serialize(&value).expect("Failed to serialize");
        self.db.put(key_bytes, value_bytes)?;
        Ok(())
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

    fn get_next_entry(&self, start_key: &[u8]) -> (Vec<u8>, Vec<u8>) {
        // Create an iterator starting from the key after the specified start_key.
        let mut iter = self.db.iterator(IteratorMode::From(start_key, rocksdb::Direction::Forward));

        // Iterate to get the next entry.
        let boxes = iter.next().expect("Something wrong with iter.next function").unwrap();
        let (key, value) = boxes;
        let new_key = key.into_vec();
        println!("key the get_next_entry: {:?}", new_key);
        return (new_key, value.into_vec());
    }
}

// fn delete_all_data() {
// let path = Path::new("epool");
// let db = DB::open_default(&path).expect("Failed to open database");
// Create an iterator starting at the first key.
// let iter = db.iterator(IteratorMode::Start);
//
// Iterate through all key-value pairs and print them.
// for result in iter {
// match result {
// Ok((key, value)) => {
// println!("display_all_data: key: {:?} value {:?}", key, value);
// let deleted = db.delete(key);
// match deleted {
// Ok(()) => {
// println!("Deleted successfully")
// }
// Err(err) => {
// eprintln!("There is an error! {:?}", err);
// }
// }
// }
// Err(err) => {
// eprintln!("There is an error! {:?}", err);
// }
// }
// }
// }
// fn display_all_data() {
//     let path = Path::new("epool");
//     let db = DB::open_default(&path).expect("Failed to open database");
//     // Create an iterator starting at the first key.
//     let iter = db.iterator(IteratorMode::Start);

//     // Iterate through all key-value pairs and print them.
//     for result in iter {
//         match result {
//             Ok((key, value)) => {
//                 // println!("display_all_data: key: {:?} value {:?}", key, value);
//                 println!("display_all_data: key: {:?}", key);
//             }
//             Err(err) => {
//                 eprintln!("There is an error! {:?}", err);
//             }
//         }
//     }
// }

// fn get_next_entry(start_key: &[u8]) -> (Vec<u8>, Vec<u8>) {
// Create an iterator starting from the key after the specified start_key.
// let path = Path::new("epool");
// let db = DB::open_default(&path).expect("Failed to open database");
//
// let mut iter = db.iterator(IteratorMode::From(start_key, rocksdb::Direction::Forward));
//
// Iterate to get the next entry.
// let boxes = iter.next().expect("Something wrong with iter.next function").unwrap();
// let (key, value) = boxes;
// let new_key = key.into_vec();
// println!("key the get_next_entry: {:?}", new_key);
// return (new_key, value.into_vec());
// }
//

// fn get_next_entry(db: &DB, start_key: &[u8]) -> Result<Option<(Box<[u8]>, Box<[u8]>)>, Error> {
// Create an iterator starting from the key after the specified start_key.
// let path = Path::new("epool");
// let db = DB::open_default(&path).expect("Failed to open database");
//
// let mut iter = db.iterator(IteratorMode::From(start_key, rocksdb::Direction::Forward));
//
// Iterate to get the next entry.
// if let Some(Ok((key, value))) = iter.next() {
// return Ok(Some((key.to_vec().into_boxed_slice(), value.to_vec().into_boxed_slice())));
// }
//
// Ok(None) // Return Ok(None) if no next entry is found.
// }

fn encode_data_to_base64(original: &str) -> String {
    // Convert string to bytes
    let bytes = original.as_bytes();
    // Convert bytes to base64
    let base64_str: String = general_purpose::STANDARD.encode(&bytes);
    base64_str
}

pub fn submit_block_to_db(block_height: u64, txs: Vec<u8>) {
    println!("submit_block_to_db: key: {:?}", block_height);

    // Open or create a RocksDB database.
    let path = Path::new("epool");
    let db = DB::open_default(&path).expect("Failed to open database");
    db.put(block_height.to_ne_bytes(), txs).expect("Failed to put tx into RocksDB");
    println!("THIS IS BLOCK HEIGHT FROM submit_block_to_db: {:?}", block_height.to_ne_bytes());
    db.put("sync_target".as_bytes(), serialize(&block_height).expect("Failed to serialize"))
        .expect("Failed to put tx into RocksDB");
}

pub fn submit_to_db(key: &[u8], value: Vec<u8>) {
    println!("submit_to_db: key: {:?} value: {:?}", key, value);
    // Open or create a RocksDB database.
    let path = Path::new("epool");
    let db = DB::open_default(&path).expect("Failed to open database");
    db.put(key, value).expect("Failed to put tx into RocksDB");
}

pub fn retrieve_from_db(key: &[u8]) -> Vec<u8> {
    let path = Path::new("epool");
    let db = DB::open_default(&path).expect("Failed to open database");
    let value = db.get(key).expect("msg");
    let result = match value {
        Some(val) => val,
        None => Vec::new(), // Provide a default value (empty vector) for the None arm
    };
    println!("THIS IS BLOCK HEIGHT FROM retrieve_from_db: {:?}", result.as_slice());

    result
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

// From_ne_bytes. In Rust we have standard library functions to convert from bytes to integers and
// back again. The from_le_bytes, from_ne_bytes and from_be_bytes functions can be used. To convert
// from an integer back into an array of bytes, we can use functions like to_ne_bytes. The correct
// Endianness must be selected.

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

    match my_db.write("sync", "Hello World") {
        Ok(()) => {
            println!("Successfully wrote to DB")
        }
        Err(err) => {
            eprintln!("Error in writing to DB: {}", err);
        }
    };

    my_db.clear_db();
    println!("Clearance");

    my_db.display_db();
    println!("First display after writing");

    let three_seconds = time::Duration::from_millis(3000);

    let mut txs: Vec<u8>;
    let mut sync_target_bin: Vec<u8>;
    let mut sync_target: u64 = 0;
    let mut sync_bin: Vec<u8>;
    let mut sync: u64 = 0;
    let mut block_height: String = "".to_string();

    my_db.write("sync", sync);
    my_db.write("sync_target", sync_target);

    // Create the runtime
    let rt = Runtime::new().unwrap();

    // loop {
    //     println!("SLEEPING FOR 3 SECONDS");
    //     thread::sleep(three_seconds);
    //     sync_target_bin = retrieve_from_db("sync_target".as_bytes());
    //     sync_bin = retrieve_from_db("sync".as_bytes());
    //     println!("sync_target_bin: {:?}, sync_bin: {:?}", sync_target_bin, sync_bin);
    //     sync_target = deserialize(&sync_target_bin).expect("Failed to deserialize");
    //     sync = deserialize(&sync_bin).expect("Failed to deserialize");

    //     println!("sync_target: {:?} and sync {:?}", sync_target, sync);
    //     if sync_target != sync {
    //         display_all_data();

    //         println!("this is sync_bin inside if: {:?}", sync);
    //         println!("this is sync_target inside if: {:?}", sync_target);

    //         let (key, value) = get_next_entry(&sync_bin);
    //         let deser_key: u64 = deserialize(&key).expect("Failed to deserialize");
    //         txs = retrieve_from_db(&serialize(&deser_key).expect("Failed to serialize"));
    //         println!("these are the txs: {:?}", txs.len());
    //         let s = match str::from_utf8(&txs) {
    //             Ok(v) => v,
    //             Err(e) => panic!("Invalid UTF-8 sequence: {}", e),
    //         };
    //         println!("JUST CASUALLY CHECKING AROUND AND THIS IS s: {:?}", s);
    //         rt.block_on(async {
    //             block_height = submit_to_da(&encode_data_to_base64(s)).await;
    //             println!("this is the block height from DA: {}", block_height);
    //         });
    //         println!("try to submit block no. {:?}", key);
    //         if !(block_height.len() == 0) {
    //             submit_to_db("sync".as_bytes(), key.clone());
    //             println!("last synced block is updated to {:?}", &key);
    //         }
    //     }
    // }
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
