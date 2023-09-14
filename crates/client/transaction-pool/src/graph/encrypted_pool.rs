//! Substrate transaction pool implementation.
#![warn(missing_docs)]
#![warn(unused_extern_crates)]

use std::collections::HashMap;

use mp_starknet::transaction::types::{EncryptedInvokeTransaction, Transaction};
// use sp_runtime::traits::Block as BlockT;

// pub struct NewBlock(Box<dyn BlockT>);

// impl NewBlock {
//     fn new<B: BlockT>(block: B) -> Self {
//         Self(Box::new(block))
//     }
// }

#[derive(Debug, Clone, Default)]
/// Txs struct
/// 1 Txs for 1 block
/// * `encrypted_pool`: Map store encrypted tx
/// * `key_received`: Map store specific order's key receivement.
/// * `decrypted_cnt`: decrypted tx count
/// * `order`: current order
pub struct Txs {
    /// store encrypted tx
    encrypted_pool: HashMap<u64, EncryptedInvokeTransaction>,
    /// store temporary encrypted tx
    temporary_pool: Vec<(u64, Transaction)>,
    /// store specific order's key receivement.
    key_received: HashMap<u64, bool>,
    /// decrypted tx count
    decrypted_cnt: u64,
    /// current order
    order: u64,
    /// close flag
    closed: bool,
}

impl Txs {
    /// new
    pub fn new() -> Self {
        Self {
            encrypted_pool: HashMap::default(),
            temporary_pool: Vec::default(),
            key_received: HashMap::default(),
            decrypted_cnt: 0,
            order: 0,
            closed: false,
        }
    }

    /// add encrypted tx on Txs
    pub fn set(&mut self, encrypted_invoke_transaction: EncryptedInvokeTransaction) -> u64 {
        self.encrypted_pool.insert(self.order, encrypted_invoke_transaction);
        self.key_received.insert(self.order, false);
        self.increase_order();
        self.order - 1
    }

    /// get encrypted tx for order
    pub fn get(&self, order: u64) -> Result<EncryptedInvokeTransaction, &str> {
        match self.encrypted_pool.get(&order) {
            Some(item) => Ok(item.clone()),
            None => Err("get not exist tx from vector"),
        }
    }

    /// is close
    pub fn is_closed(&self) -> bool {
        // println!("is closed {}", self.closed);
        self.closed
    }

    /// close
    pub fn close(&mut self) -> bool {
        self.closed = true;
        self.closed
    }

    /// add tx to temporary pool
    pub fn add_tx_to_temporary_pool(&mut self, order: u64, tx: Transaction) {
        self.temporary_pool.push((order, tx));
    }

    /// get tx from temporary pool
    pub fn get_tx_from_temporary_pool(&mut self, index: usize) -> (u64, Transaction) {
        match self.temporary_pool.get(index) {
            Some(tx) => tx.clone(),
            None => panic!("aaaaaaak"),
        }
    }

    /// get temporary pool length
    pub fn temporary_pool_len(&mut self) -> usize {
        self.temporary_pool.len()
    }

    /// get temporary pool
    pub fn get_temporary_pool(&self) -> Vec<(u64, Transaction)> {
        self.temporary_pool.clone()
    }

    /// increase order
    /// not only for set new encrypted tx
    /// but also for declare tx, deploy account tx
    pub fn increase_order(&mut self) -> u64 {
        self.order = self.order + 1;
        self.order
    }

    /// order getter
    pub fn get_order(&self) -> u64 {
        self.order
    }

    /// get encrypted tx count
    /// it's not order
    pub fn get_tx_cnt(&self) -> u64 {
        self.encrypted_pool.len() as u64
    }

    /// increase decrypted tx count
    pub fn increase_decrypted_cnt(&mut self) -> u64 {
        self.decrypted_cnt = self.decrypted_cnt + 1;
        self.decrypted_cnt
    }

    /// get decrypted tx count
    pub fn get_decrypted_cnt(&self) -> u64 {
        self.decrypted_cnt
    }

    /// update key received information
    pub fn update_key_received(&mut self, order: u64) {
        self.key_received.insert(order, true);
    }

    /// get key received information
    pub fn get_key_received(&self, order: u64) -> bool {
        match self.key_received.get(&order) {
            Some(received) => received.clone(),
            None => false,
        }
    }
}

/// epool
#[derive(Debug, Clone, Default)]
/// EncryptedPool struct
/// 1 epool for node
/// * `txs`: Map of Txs, key:value = block_height:Txs
/// * `enabled`: epool enabler. if whole part is splitted by package. it have to be removed.
pub struct EncryptedPool {
    /// Map of Txs, key:value = block_height:Txs
    txs: HashMap<u64, Txs>,
    /// epool enabler. if whole part is splitted by package. it have to be removed.
    enabled: bool,
}

impl EncryptedPool {
    /// enable epool
    pub fn enable_encrypted_mempool(&mut self) {
        self.enabled = true;
    }

    /// disable epool
    pub fn disable_encrypted_mempool(&mut self) {
        self.enabled = false;
    }

    /// check epool is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// check epool is disabled
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }

    /// new epool
    pub fn new(encrypted_mempool: bool) -> Self {
        Self { txs: HashMap::default(), enabled: encrypted_mempool }
    }

    /// add new Txs for block_height
    pub fn new_block(&mut self, block_height: u64) -> Txs {
        self.txs.insert(block_height, Txs::new());
        self.txs.get(&block_height).unwrap().clone()
    }

    /// txs exist
    pub fn exist(&self, block_height: u64) -> bool {
        match self.txs.get(&block_height) {
            Some(_) => true,
            None => false,
        }
    }

    /// get txs
    pub fn get_txs(&self, block_height: u64) -> Result<Txs, &str> {
        match self.txs.get(&block_height) {
            Some(txs) => Ok(txs.clone()),
            None => Err("get not exist tx from map"),
        }
    }

    /// add encrypted tx by block_height
    pub fn set(&mut self, block_height: u64, encrypted_invoke_transaction: EncryptedInvokeTransaction) -> u64 {
        println!("set encrypted tx on {}", block_height);
        match self.txs.get_mut(&block_height) {
            Some(txs) => {
                println!("txs exist add tx");
                txs.set(encrypted_invoke_transaction)
            }
            None => {
                println!("txs not exist");
                self.txs.insert(block_height, Txs::new());
                match self.txs.get(&block_height) {
                    Some(txs) => println!("exist"),
                    None => println!("not exist"),
                };
                match self.txs.get_mut(&block_height) {
                    Some(txs) => txs.set(encrypted_invoke_transaction),
                    None => panic!("???"),
                };
                0
            }
        }
    }

    /// get encrypted tx by block_height and order
    pub fn get(&self, block_height: u64, order: u64) -> Result<&EncryptedInvokeTransaction, &str> {
        match self.txs.get(&block_height) {
            Some(txs) => match txs.encrypted_pool.get(&order) {
                Some(tx) => Ok(tx),
                None => Err("get not exist tx from map"),
            },
            None => Err("get not exist tx from map"),
        }
    }

    /// block is making so add tx to temporary pool
    pub fn add_tx_to_temporary_pool(&mut self, block_height: u64, order: u64, tx: Transaction) -> Result<(), &str> {
        match self.txs.get_mut(&block_height) {
            Some(txs) => Ok(txs.clone().add_tx_to_temporary_pool(order, tx)),
            None => Err("add_tx_to_temporary_pool not exist tx from map"),
        }
    }

    /// get tx from temporary_pool before making block
    pub fn get_tx_from_temporary_pool(&mut self, block_height: u64, index: usize) -> Result<(u64, Transaction), &str> {
        match self.txs.get(&block_height) {
            Some(txs) => Ok(txs.clone().get_tx_from_temporary_pool(index)),
            None => Err("get_tx_to_temporary_pool not exist tx from map"),
        }
    }

    // /// get temporary poll length
    // pub fn temporary_pool_len(&mut self, block_height: u64) -> Result<usize, &str> {
    //     match self.txs.get(&block_height) {
    //         Some(txs) => Ok(txs.clone().temporary_pool_len()),
    //         None => Err("temporary_pool_len not exist tx from map"),
    //     }
    // }

    /// is close
    pub fn is_closed(&self, block_height: u64) -> Result<bool, &str> {
        match self.txs.get(&block_height) {
            Some(txs) => Ok(txs.clone().is_closed()),
            None => Err("is_closed not exist tx from map"),
        }
    }

    /// close
    pub fn close(&mut self, block_height: u64) -> Result<bool, &str> {
        match self.txs.get_mut(&block_height) {
            Some(txs) => {
                // println!("close!");
                Ok(txs.close())
            }
            None => Err("not exist? cannot close"),
        }
    }

    /// increase order for block_height
    pub fn increase_order(&mut self, block_height: u64) -> u64 {
        match self.txs.get_mut(&block_height) {
            Some(txs) => txs.increase_order(),
            None => {
                let mut txs = self.new_block(block_height);
                txs.increase_order()
            }
        }
    }

    /// order getter
    pub fn get_order(&self, block_height: u64) -> u64 {
        match self.txs.get(&block_height) {
            Some(txs) => txs.get_order(),
            None => panic!("no txs on {}", block_height),
        }
    }

    /// get encrypted tx count (not order)
    pub fn get_tx_cnt(&self, block_height: u64) -> u64 {
        match self.txs.get(&block_height) {
            Some(txs) => txs.get_tx_cnt(),
            None => 0,
        }
    }

    /// increase decrypted tx count
    pub fn increase_decrypted_cnt(&mut self, block_height: u64) -> u64 {
        println!("increase key received count on {}", block_height);
        match self.txs.get_mut(&block_height) {
            Some(txs) => txs.increase_decrypted_cnt(),
            None => panic!("no txs on {}", block_height),
        }
    }

    /// get decrypted tx count
    pub fn get_decrypted_cnt(&self, block_height: u64) -> u64 {
        match self.txs.get(&block_height) {
            Some(txs) => txs.get_decrypted_cnt(),
            None => 0,
        }
    }

    /// update key received information
    pub fn update_key_received(&mut self, block_height: u64, order: u64) {
        match self.txs.get_mut(&block_height) {
            Some(txs) => txs.update_key_received(order),
            None => panic!("not exist txs"),
        }
    }

    /// get key received information
    pub fn get_key_received(&self, block_height: u64, order: u64) -> bool {
        println!("find key received for {}:{}", block_height, order);
        match self.txs.get(&block_height) {
            Some(txs) => txs.get_key_received(order),
            None => panic!("no txs on {}", block_height),
        }
    }

    /// get encrypted pool
    pub fn get_encrypted_tx_pool(&self, block_height: u64) -> Vec<EncryptedInvokeTransaction> {
        match self.txs.get(&block_height) {
            None => [].to_vec(),
            Some(txs) => txs.encrypted_pool.values().cloned().collect(),
        }
    }

    /// init tx pool of block_height
    pub fn init_tx_pool(&mut self, block_height: u64) {
        self.txs.remove(&block_height);
    }

    /// get length of encrypted tx count of block_height
    pub fn len(&self, block_height: u64) -> usize {
        match self.txs.get(&block_height) {
            None => 0,
            Some(txs) => txs.encrypted_pool.values().cloned().collect::<Vec<_>>().len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EncryptedPool;

    #[test]
    fn first_test() {
        let mut epool = EncryptedPool::default();

        // assert_eq!(ready.get().count(), 1);
    }
}
