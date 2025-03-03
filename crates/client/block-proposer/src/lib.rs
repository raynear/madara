//! Block proposer implementation.
//! This crate implements the [`sp_consensus::Proposer`] trait.
//! It is used to build blocks for the block authoring node.
//! The block authoring node is the node that is responsible for building new blocks.
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time;
use std::time::{Duration, UNIX_EPOCH};

use codec::Encode;
use futures::channel::oneshot;
use futures::future::{Future, FutureExt};
use futures::{future, select};
use log::{debug, error, info, trace, warn};
use mc_rpc::submit_extrinsic_with_order;
use mc_transaction_pool::decryptor::Decryptor;
use mc_transaction_pool::EncryptedTransactionPool;
use mp_starknet::execution::types::Felt252Wrapper;
use mp_starknet::transaction::types::{EncryptedInvokeTransaction, InvokeTransaction, Transaction as MPTransaction, TxType};
use pallet_starknet::runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use prometheus_endpoint::Registry as PrometheusRegistry;
use sc_block_builder::{BlockBuilderApi, BlockBuilderProvider};
use sc_client_api::backend;
use sc_proposer_metrics::{EndProposingReason, MetricsLink as PrometheusMetrics};
use sc_transaction_pool_api::{InPoolTransaction, TransactionSource};
use sp_api::{ApiExt, ProvideRuntimeApi};
use sp_blockchain::ApplyExtrinsicFailed::Validity;
use sp_blockchain::Error::ApplyExtrinsicFailed;
use sp_blockchain::HeaderBackend;
use sp_consensus::{DisableProofRecording, ProofRecording, Proposal};
use sp_core::traits::SpawnNamed;
use sp_inherents::InherentData;
use sp_runtime::generic::BlockId as SPBlockId;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
use sp_runtime::{Digest, Percent, SaturatedConversion};
use tokio;

/// Default block size limit in bytes used by [`Proposer`].
///
/// Can be overwritten by [`ProposerFactory::set_default_block_size_limit`].
///
/// Be aware that there is also an upper packet size on what the networking code
/// will accept. If the block doesn't fit in such a package, it can not be
/// transferred to other nodes.
pub const DEFAULT_BLOCK_SIZE_LIMIT: usize = 4 * 1024 * 1024 + 512;
/// Default value for `soft_deadline_percent` used by [`Proposer`].
/// `soft_deadline_percent` value is used to compute soft deadline during block production.
/// The soft deadline indicates where we should stop attempting to add transactions
/// to the block, which exhaust resources. After soft deadline is reached,
/// we switch to a fixed-amount mode, in which after we see `MAX_SKIPPED_TRANSACTIONS`
/// transactions which exhaust resources, we will conclude that the block is full.
const DEFAULT_SOFT_DEADLINE_PERCENT: Percent = Percent::from_percent(80);

const LOG_TARGET: &str = "block-proposer";

/// [`Proposer`] factory.
pub struct ProposerFactory<A, B, C, PR> {
    spawn_handle: Box<dyn SpawnNamed>,
    /// The client instance.
    client: Arc<C>,
    /// The transaction pool.
    transaction_pool: Arc<A>,
    /// Prometheus Link,
    metrics: PrometheusMetrics,
    /// The default block size limit.
    ///
    /// If no `block_size_limit` is passed to [`sp_consensus::Proposer::propose`], this block size
    /// limit will be used.
    default_block_size_limit: usize,
    /// Soft deadline percentage of hard deadline.
    ///
    /// The value is used to compute soft deadline during block production.
    /// The soft deadline indicates where we should stop attempting to add transactions
    /// to the block, which exhaust resources. After soft deadline is reached,
    /// we switch to a fixed-amount mode, in which after we see `MAX_SKIPPED_TRANSACTIONS`
    /// transactions which exhaust resources, we will conclude that the block is full.
    soft_deadline_percent: Percent,
    /// phantom member to pin the `Backend`/`ProofRecording` type.
    _phantom: PhantomData<(B, PR)>,
}

impl<A, B, C> ProposerFactory<A, B, C, DisableProofRecording> {
    /// Create a new proposer factory.
    ///
    /// Proof recording will be disabled when using proposers built by this instance to build
    /// blocks.
    pub fn new(
        spawn_handle: impl SpawnNamed + 'static,
        client: Arc<C>,
        transaction_pool: Arc<A>,
        prometheus: Option<&PrometheusRegistry>,
    ) -> Self {
        ProposerFactory {
            spawn_handle: Box::new(spawn_handle),
            transaction_pool,
            metrics: PrometheusMetrics::new(prometheus),
            default_block_size_limit: DEFAULT_BLOCK_SIZE_LIMIT,
            soft_deadline_percent: DEFAULT_SOFT_DEADLINE_PERCENT,
            client,
            _phantom: PhantomData,
        }
    }
}

impl<A, B, C, PR> ProposerFactory<A, B, C, PR> {
    /// Set the default block size limit in bytes.
    ///
    /// The default value for the block size limit is:
    /// [`DEFAULT_BLOCK_SIZE_LIMIT`].
    ///
    /// If there is no block size limit passed to [`sp_consensus::Proposer::propose`], this value
    /// will be used.
    pub fn set_default_block_size_limit(&mut self, limit: usize) {
        self.default_block_size_limit = limit;
    }

    /// Set soft deadline percentage.
    ///
    /// The value is used to compute soft deadline during block production.
    /// The soft deadline indicates where we should stop attempting to add transactions
    /// to the block, which exhaust resources. After soft deadline is reached,
    /// we switch to a fixed-amount mode, in which after we see `MAX_SKIPPED_TRANSACTIONS`
    /// transactions which exhaust resources, we will conclude that the block is full.
    ///
    /// Setting the value too low will significantly limit the amount of transactions
    /// we try in case they exhaust resources. Setting the value too high can
    /// potentially open a DoS vector, where many "exhaust resources" transactions
    /// are being tried with no success, hence block producer ends up creating an empty block.
    pub fn set_soft_deadline(&mut self, percent: Percent) {
        self.soft_deadline_percent = percent;
    }
}

impl<B, Block, C, A, PR> ProposerFactory<A, B, C, PR>
where
    A: EncryptedTransactionPool<Block = Block> + 'static,
    B: backend::Backend<Block> + Send + Sync + 'static,
    Block: BlockT,
    C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
    C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>> + BlockBuilderApi<Block>,
{
    fn init_with_now(
        &mut self,
        parent_header: &<Block as BlockT>::Header,
        now: Box<dyn Fn() -> time::Instant + Send + Sync>,
    ) -> Proposer<B, Block, C, A, PR> {
        let parent_hash = parent_header.hash();

        info!("🩸 Starting consensus session on top of parent {:?}", parent_hash);

        let proposer = Proposer::<_, _, _, _, PR> {
            spawn_handle: self.spawn_handle.clone(),
            client: self.client.clone(),
            parent_hash,
            parent_number: *parent_header.number(),
            transaction_pool: self.transaction_pool.clone(),
            now,
            metrics: self.metrics.clone(),
            default_block_size_limit: self.default_block_size_limit,
            soft_deadline_percent: self.soft_deadline_percent,
            _phantom: PhantomData,
        };

        proposer
    }
}

impl<A, B, Block, C, PR> sp_consensus::Environment<Block> for ProposerFactory<A, B, C, PR>
where
    A: EncryptedTransactionPool<Block = Block> + 'static,
    B: backend::Backend<Block> + Send + Sync + 'static,
    Block: BlockT,
    C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
    C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
        + BlockBuilderApi<Block>
        + StarknetRuntimeApi<Block>
        + ConvertTransactionRuntimeApi<Block>,
    PR: ProofRecording,
{
    type CreateProposer = future::Ready<Result<Self::Proposer, Self::Error>>;
    type Proposer = Proposer<B, Block, C, A, PR>;
    type Error = sp_blockchain::Error;

    fn init(&mut self, parent_header: &<Block as BlockT>::Header) -> Self::CreateProposer {
        future::ready(Ok(self.init_with_now(parent_header, Box::new(time::Instant::now))))
    }
}

/// The proposer logic.
pub struct Proposer<B, Block: BlockT, C, A: EncryptedTransactionPool, PR> {
    spawn_handle: Box<dyn SpawnNamed>,
    client: Arc<C>,
    parent_hash: Block::Hash,
    parent_number: <<Block as BlockT>::Header as HeaderT>::Number,
    transaction_pool: Arc<A>,
    now: Box<dyn Fn() -> time::Instant + Send + Sync>,
    metrics: PrometheusMetrics,
    default_block_size_limit: usize,
    soft_deadline_percent: Percent,
    _phantom: PhantomData<(B, PR)>,
}

impl<A, B, Block, C, PR> sp_consensus::Proposer<Block> for Proposer<B, Block, C, A, PR>
where
    A: EncryptedTransactionPool<Block = Block> + 'static,
    B: backend::Backend<Block> + Send + Sync + 'static,
    Block: BlockT,
    C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
    C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
        + BlockBuilderApi<Block>
        + StarknetRuntimeApi<Block>
        + ConvertTransactionRuntimeApi<Block>,
    PR: ProofRecording,
{
    type Transaction = backend::TransactionFor<B, Block>;
    type Proposal =
        Pin<Box<dyn Future<Output = Result<Proposal<Block, Self::Transaction, PR::Proof>, Self::Error>> + Send>>;
    type Error = sp_blockchain::Error;
    type ProofRecording = PR;
    type Proof = PR::Proof;

    fn propose(
        self,
        inherent_data: InherentData,
        inherent_digests: Digest,
        max_duration: time::Duration,
        block_size_limit: Option<usize>,
    ) -> Self::Proposal {
        let (tx, rx) = oneshot::channel();
        let spawn_handle = self.spawn_handle.clone();

        spawn_handle.spawn_blocking(
            "madara-block-proposer",
            None,
            Box::pin(async move {
                // Leave some time for evaluation and block finalization (20%)
                // and some time for block production (80%).
                // We need to benchmark and tune this value.
                // Open question: should we make this configurable?
                let deadline = (self.now)() + max_duration - max_duration / 5;
                let res = self.propose_with(inherent_data, inherent_digests, deadline, block_size_limit).await;
                if tx.send(res).is_err() {
                    trace!("Could not send block production result to proposer!");
                }
            }),
        );

        async move { rx.await? }.boxed()
    }
}

/// If the block is full we will attempt to push at most
/// this number of transactions before quitting for real.
/// It allows us to increase block utilization.
const MAX_SKIPPED_TRANSACTIONS: usize = 8;

impl<A, B, Block, C, PR> Proposer<B, Block, C, A, PR>
where
    A: EncryptedTransactionPool<Block = Block> + 'static,
    B: backend::Backend<Block> + Send + Sync + 'static,
    Block: BlockT,
    C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
    C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
        + BlockBuilderApi<Block>
        + StarknetRuntimeApi<Block>
        + ConvertTransactionRuntimeApi<Block>,
    PR: ProofRecording,
{
    /// Propose a new block.
    ///
    /// # Arguments
    /// * `inherents` - The inherents to include in the block.
    /// * `inherent_digests` - The inherent digests to include in the block.
    /// * `deadline` - The deadline for proposing the block.
    /// * `block_size_limit` - The maximum size of the block in bytes.
    ///
    ///
    /// The function follows these general steps:
    /// 1. Starts a timer to measure the total time it takes to create the proposal.
    /// 2. Initializes a new block at the parent hash with the given inherent digests.
    /// 3. Iterates over the inherents and pushes them into the block builder. Handles any potential
    /// errors.
    /// 4. Sets up the soft deadline and starts the block timer.
    /// 5. Gets an iterator over the pending transactions and iterates over them.
    /// 6. Checks the deadline and handles the case when the deadline is reached.
    /// 7. Checks the block size limit and handles cases where transactions would cause the block to
    /// exceed the limit.
    /// 8. Attempts to push the transaction into the block and handles any
    /// potential errors.
    /// 9. If the block size limit was reached without adding any transaction,
    /// it logs a warning.
    /// 10. Removes invalid transactions from the pool.
    /// 11. Builds the block and updates the metrics.
    /// 12. Converts the storage proof to the required format.
    /// 13. Measures the total time it took to create the proposal and updates the corresponding
    /// metric.
    /// 14. Returns a new `Proposal` with the block, proof, and storage changes.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The block cannot be created at the parent hash.
    /// - Any of the inherents cannot be pushed into the block builder.
    /// - The block cannot be built.
    /// - The storage proof cannot be converted into the required format.
    async fn propose_with(
        self,
        inherent_data: InherentData,
        inherent_digests: Digest,
        deadline: time::Instant,
        block_size_limit: Option<usize>,
    ) -> Result<Proposal<Block, backend::TransactionFor<B, Block>, PR::Proof>, sp_blockchain::Error> {
        // Start the timer to measure the total time it takes to create the proposal.
        let propose_with_timer = time::Instant::now();

        // Initialize a new block builder at the parent hash with the given inherent digests.
        let mut block_builder = self.client.new_block_at(self.parent_hash, inherent_digests, PR::ENABLED)?;

        self.apply_inherents(&mut block_builder, inherent_data)?;

        let block_timer = time::Instant::now();

        // Apply transactions and record the reason why we stopped.
        let end_reason = self.apply_extrinsics(&mut block_builder, deadline, block_size_limit).await?;

        // Build the block.
        let (block, storage_changes, proof) = block_builder.build()?.into_inner();

        // Measure the total time it took to build the block.
        let block_took = block_timer.elapsed();

        // Convert the storage proof into the required format.
        let proof = PR::into_proof(proof).map_err(|e| sp_blockchain::Error::Application(Box::new(e)))?;

        // Print the summary of the proposal.
        self.print_summary(&block, end_reason, block_took, propose_with_timer.elapsed());

        Ok(Proposal { block, proof, storage_changes })
    }

    /// Apply all inherents to the block.
    /// This function will return an error if any of the inherents cannot be pushed into the block
    /// builder. It will also update the metrics.
    /// # Arguments
    /// * `block_builder` - The block builder to push the inherents into.
    /// * `inherent_data` - The inherents to push into the block builder.
    /// # Returns
    /// This function will return `Ok(())` if all inherents were pushed into the block builder.
    /// # Errors
    /// This function will return an error if any of the inherents cannot be pushed into the block
    /// builder.
    fn apply_inherents(
        &self,
        block_builder: &mut sc_block_builder::BlockBuilder<'_, Block, C, B>,
        inherent_data: InherentData,
    ) -> Result<(), sp_blockchain::Error> {
        let create_inherents_start = time::Instant::now();
        let inherents = block_builder.create_inherents(inherent_data)?;
        let create_inherents_end = time::Instant::now();

        self.metrics.report(|metrics| {
            metrics
                .create_inherents_time
                .observe(create_inherents_end.saturating_duration_since(create_inherents_start).as_secs_f64());
        });

        for inherent in inherents {
            match block_builder.push(inherent) {
                Err(ApplyExtrinsicFailed(Validity(e))) if e.exhausted_resources() => {
                    warn!(target: LOG_TARGET, "⚠️  Dropping non-mandatory inherent from overweight block.")
                }
                Err(ApplyExtrinsicFailed(Validity(e))) if e.was_mandatory() => {
                    error!("❌️ Mandatory inherent extrinsic returned error. Block cannot be produced.");
                    return Err(ApplyExtrinsicFailed(Validity(e)));
                }
                Err(e) => {
                    warn!(target: LOG_TARGET, "❗️ Inherent extrinsic returned unexpected error: {}. Dropping.", e);
                }
                Ok(_) => {}
            }
        }
        Ok(())
    }

    /// Apply as many extrinsics as possible to the block.
    /// This function will return an error if the block cannot be built.
    /// # Arguments
    /// * `block_builder` - The block builder to push the extrinsics into.
    /// * `deadline` - The deadline to stop applying extrinsics.
    /// * `block_size_limit` - The maximum size of the block.
    /// # Returns
    /// The reason why we stopped applying extrinsics.
    /// # Errors
    /// This function will return an error if the block cannot be built.
    async fn apply_extrinsics(
        &self,
        block_builder: &mut sc_block_builder::BlockBuilder<'_, Block, C, B>,
        deadline: time::Instant,
        block_size_limit: Option<usize>,
    ) -> Result<EndProposingReason, sp_blockchain::Error> {
        let epool = self.transaction_pool.encrypted_pool().clone();
        let block_height = self.parent_number.to_string().parse::<u64>().unwrap() + 1;

        let enabled = {
            let lock = epool.lock().await;
            lock.is_enabled()
        };

        let using_external_decryptor = {
            let lock = epool.lock().await;
            lock.is_using_external_decryptor()
        };

        let closed = {
            let mut lock = epool.lock().await;
            let exist = lock.exist(block_height);
            if exist {
                // lock.get_txs(block_height).unwrap().is_closed()
                lock.txs.get_mut(&block_height).unwrap().is_closed()
            } else {
                lock.initialize_if_not_exist(block_height);
                println!("log1");
                let len = lock.txs.get(&block_height).unwrap().len();
                // let len = lock.get_txs(block_height).unwrap().len();
                println!("{}", len);
                false
            }
        };

        if enabled && !closed {
            // add temporary pool tx to pool
            let mut temporary_pool: Vec<(u64, MPTransaction)> = vec![];
            {
                let mut lock = epool.lock().await;
                println!("close on {}", block_height);
                let txs = lock.txs.get_mut(&block_height).unwrap();
                // let mut txs = lock.get_txs(block_height).unwrap();

                let tx_cnt = txs.get_tx_cnt();
                let dec_cnt = txs.get_decrypted_cnt();
                println!("test1: {}:{}", tx_cnt, dec_cnt);

                temporary_pool = txs.get_temporary_pool();
            }

            {
                let mut lock = epool.lock().await;

                let _ = lock.close(block_height);
            }

            let best_block_hash = self.client.info().best_hash;
            for (order, transaction) in temporary_pool {
                let extrinsic = self
                    .client
                    .runtime_api()
                    .convert_transaction(best_block_hash, transaction, TxType::Invoke)
                    .expect("convert_transaction")
                    .expect("runtime_api");
                let _ = self
                    .transaction_pool
                    .clone()
                    .submit_one_with_order(
                        &SPBlockId::hash(best_block_hash),
                        TransactionSource::External,
                        extrinsic,
                        order,
                    )
                    .await;
            }

            let cnt = {
                let lock = epool.lock().await;

                lock.txs.get(&block_height).unwrap().len() as u64
                // lock.get_txs(block_height).unwrap().len() as u64
            };

            let start = std::time::SystemTime::now();
            let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
            println!("Decrypt Start in {:?}", since_the_epoch);

            for order in 0..cnt {
                let block_height = self.parent_number.to_string().parse::<u64>().unwrap() + 1;
                let best_block_hash = self.client.info().best_hash;
                let client = self.client.clone();
                let pool = self.transaction_pool.clone();
                let chain_id = Felt252Wrapper(client.runtime_api().chain_id(best_block_hash).unwrap().into());
                let epool = self.transaction_pool.encrypted_pool().clone();
                self.spawn_handle.spawn_blocking(
                    "Decryptor",
                    None,
                    Box::pin(
                        // tokio::task::spawn(
                        async move {
                            tokio::time::sleep(Duration::from_secs(1)).await;

                            println!("stompesi - start delay function block_height {} order {}", block_height, order);

                            let encrypted_invoke_transaction: EncryptedInvokeTransaction;
                            {
                                let lock = epool.lock().await;
                                let txs = lock.txs.get(&block_height).expect("expect get txs");
                                // let txs = lock.get_txs(block_height).unwrap();
                                // println!("check key_received on block_height {} order {}", block_height, order);
                                let did_received_key = txs.get_key_received(order);

                                if did_received_key == true {
                                    println!("Received key");
                                    return;
                                }
                                println!("Not received key");
                                encrypted_invoke_transaction = txs.get(order).unwrap().clone();
                            }

                            let decryptor = Decryptor::new();
                            let invoke_tx: InvokeTransaction = if using_external_decryptor {
                                decryptor
                                    .delegate_to_decrypt_encrypted_invoke_transaction(
                                        encrypted_invoke_transaction,
                                        None,
                                    )
                                    .await
                            } else {
                                decryptor.decrypt_encrypted_invoke_transaction(encrypted_invoke_transaction, None).await
                            };
                            // println!("decrypt done on block_height {} order {}", block_height, order);

                            {
                                let mut lock = epool.lock().await;
                                // println!("check key_received on block_height {} order {}", block_height, order);
                                let did_received_key = lock.txs.get(&block_height).unwrap().get_key_received(order);
                                // let did_received_key = lock.get_txs(block_height).unwrap().get_key_received(order);

                                if did_received_key == true {
                                    println!("Received key");
                                    return;
                                }

                                lock.txs.get_mut(&block_height).unwrap().increase_decrypted_cnt();
                                // lock.get_txs(block_height).unwrap().clone().
                                // increase_decrypted_cnt();
                            }

                            let end = std::time::SystemTime::now();
                            let since_the_epoch = end.duration_since(UNIX_EPOCH).expect("Time went backwards");
                            println!("Decrypt {} End in {:?}", order, since_the_epoch);

                            let transaction: MPTransaction = invoke_tx.from_invoke(chain_id);
                            let extrinsic = client
                                .runtime_api()
                                .convert_transaction(best_block_hash, transaction.clone(), TxType::Invoke)
                                .unwrap()
                                .expect("Failed to submit extrinsic");

                            submit_extrinsic_with_order(pool, best_block_hash, extrinsic, order)
                                .await
                                .expect("Failed to submit extrinsic");
                        },
                    ),
                )
            }
        }

        if enabled {
            {
                let lock = epool.lock().await;
                if lock.exist(block_height) {
                    let txs = lock.txs.get(&block_height).expect("expect get txs");
                    // let txs = lock.get_txs(block_height).unwrap();
                    let tx_cnt = txs.get_tx_cnt();
                    let dec_cnt = txs.get_decrypted_cnt();
                    let ready_cnt = self.transaction_pool.status().ready as u64;
                    println!("{} waiting {}:{}:{}", block_height, tx_cnt, dec_cnt, ready_cnt);
                    if !(tx_cnt == dec_cnt && dec_cnt == ready_cnt) {
                        return Err(sp_blockchain::Error::TransactionPoolNotReady);
                    }
                }
            }
        }

        // proceed with transactions
        // We calculate soft deadline used only in case we start skipping transactions.
        let now = (self.now)();
        let left = deadline.saturating_duration_since(now);
        let left_micros: u64 = left.as_micros().saturated_into();
        let soft_deadline = now + time::Duration::from_micros(self.soft_deadline_percent.mul_floor(left_micros));
        let mut skipped = 0;
        let mut unqueue_invalid = Vec::new();

        let mut t1 = self.transaction_pool.ready_at(self.parent_number).fuse();
        let mut t2 = futures_timer::Delay::new(deadline.saturating_duration_since((self.now)()) / 8).fuse();

        let mut pending_iterator = select! {
            res = t1 => res,
            _ = t2 => {
                warn!(target: LOG_TARGET,
                    "Timeout fired waiting for transaction pool at block #{}. \
                    Proceeding with production.",
                    self.parent_number,
                );
                self.transaction_pool.ready()
            },
        };

        let block_size_limit = block_size_limit.unwrap_or(self.default_block_size_limit);

        debug!(target: LOG_TARGET, "Attempting to push transactions from the pool.");
        debug!(target: LOG_TARGET, "Pool status: {:?}", self.transaction_pool.status());
        let mut transaction_pushed = false;

        // {
        //     let lock = epool.lock().await;
        //     if lock.is_enabled() {
        //         let block_height = self.parent_number.to_string().parse::<u64>().unwrap() + 1;
        //          let encrypted_tx_pool_size: usize = lock.len(block_height);

        //         if encrypted_tx_pool_size > 0 {
        //             let encrypted_invoke_transactions = lock.get_encrypted_tx_pool(block_height);

        //             let data_for_da: String =
        // serde_json::to_string(&encrypted_invoke_transactions).unwrap();             //
        // println!("this is the : {:?}", data_for_da);             let encoded_data_for_da =
        // encode_data_to_base64(&data_for_da);             // println!("this is the
        // encoded_data_for_da: {:?}", encoded_data_for_da);
        // submit_to_da(&encoded_data_for_da);             // let da_block_height =
        // submit_to_da(&encoded_data_for_da).await;             // println!("this is the
        // block_height: {}", da_block_height);         }
        //         // lock.init_tx_pool(block_height);
        //     }
        // }

        // input pool data to DA
        let end_reason = loop {
            let pending_tx = if let Some(pending_tx) = pending_iterator.next() {
                pending_tx
            } else {
                break EndProposingReason::NoMoreTransactions;
            };

            let now = (self.now)();
            if now > deadline {
                debug!(
                    target: LOG_TARGET,
                    "Consensus deadline reached when pushing block transactions, proceeding with proposing."
                );
                break EndProposingReason::HitDeadline;
            }

            let pending_tx_data = pending_tx.data().clone();
            let pending_tx_hash = pending_tx.hash().clone();

            let block_size = block_builder.estimate_block_size(false);
            if block_size + pending_tx_data.encoded_size() > block_size_limit {
                pending_iterator.report_invalid(&pending_tx);
                if skipped < MAX_SKIPPED_TRANSACTIONS {
                    skipped += 1;
                    debug!(
                        target: LOG_TARGET,
                        "Transaction would overflow the block size limit, but will try {} more transactions before \
                         quitting.",
                        MAX_SKIPPED_TRANSACTIONS - skipped,
                    );
                    continue;
                } else if now < soft_deadline {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction would overflow the block size limit, but we still have time before the soft \
                         deadline, so we will try a bit more."
                    );
                    continue;
                } else {
                    debug!(target: LOG_TARGET, "Reached block size limit, proceeding with proposing.");
                    break EndProposingReason::HitBlockSizeLimit;
                }
            }

            trace!(target: LOG_TARGET, "[{:?}] Pushing to the block.", pending_tx_hash);
            match sc_block_builder::BlockBuilder::push(block_builder, pending_tx_data) {
                Ok(()) => {
                    transaction_pushed = true;
                    debug!(target: LOG_TARGET, "[{:?}] Pushed to the block.", pending_tx_hash);
                }
                Err(ApplyExtrinsicFailed(Validity(e))) if e.exhausted_resources() => {
                    pending_iterator.report_invalid(&pending_tx);
                    if skipped < MAX_SKIPPED_TRANSACTIONS {
                        skipped += 1;
                        debug!(
                            target: LOG_TARGET,
                            "Block seems full, but will try {} more transactions before quitting.",
                            MAX_SKIPPED_TRANSACTIONS - skipped,
                        );
                    } else if (self.now)() < soft_deadline {
                        debug!(
                            target: LOG_TARGET,
                            "Block seems full, but we still have time before the soft deadline, so we will try a bit \
                             more before quitting."
                        );
                    } else {
                        debug!(target: LOG_TARGET, "Reached block weight limit, proceeding with proposing.");
                        break EndProposingReason::HitBlockWeightLimit;
                    }
                }
                Err(e) => {
                    pending_iterator.report_invalid(&pending_tx);
                    debug!(target: LOG_TARGET, "[{:?}] Invalid transaction: {}", pending_tx_hash, e);
                    unqueue_invalid.push(pending_tx_hash);
                }
            }
        };

        if matches!(end_reason, EndProposingReason::HitBlockSizeLimit) && !transaction_pushed {
            warn!(
                target: LOG_TARGET,
                "Hit block size limit of `{}` without including any transaction!", block_size_limit,
            );
        }

        self.transaction_pool.remove_invalid(&unqueue_invalid);
        Ok(end_reason)
    }

    /// Prints a summary and does telemetry + metrics.
    /// This is called after the block is created.
    /// # Arguments
    /// * `block` - The block that was created.
    /// * `end_reason` - The reason why we stopped adding transactions to the block.
    /// * `block_took` - The time it took to create the block.
    /// * `propose_with_took` - The time it took to propose the block.
    fn print_summary(
        &self,
        block: &Block,
        end_reason: EndProposingReason,
        block_took: time::Duration,
        propose_with_took: time::Duration,
    ) {
        let extrinsics = block.extrinsics();
        self.metrics.report(|metrics| {
            metrics.number_of_transactions.set(extrinsics.len() as u64);
            metrics.block_constructed.observe(block_took.as_secs_f64());
            metrics.report_end_proposing_reason(end_reason);
            metrics.create_block_proposal_time.observe(propose_with_took.as_secs_f64());
        });

        let extrinsics_summary = if extrinsics.is_empty() {
            "no extrinsics".to_string()
        } else {
            format!("extrinsics ({})", extrinsics.len(),)
        };

        info!(
            "🥷 Prepared block for proposing at {} ({} ms) [hash: {:?}; parent_hash: {}; {extrinsics_summary}",
            block.header().number(),
            block_took.as_millis(),
            block.header().hash(),
            block.header().parent_hash(),
        );
    }
}

#[cfg(test)]
mod tests {

    use futures::executor::block_on;
    use sc_client_api::Backend;
    use sc_transaction_pool::BasicPool;
    use sc_transaction_pool_api::{ChainEvent, MaintainedTransactionPool, TransactionSource};
    use sp_api::Core;
    use sp_blockchain::HeaderBackend;
    use sp_consensus::{BlockOrigin, Environment, Proposer};
    use sp_runtime::generic::BlockId;
    use sp_runtime::traits::NumberFor;
    use sp_runtime::Perbill;
    use substrate_test_runtime_client::prelude::*;
    use substrate_test_runtime_client::runtime::{Block as TestBlock, Extrinsic, ExtrinsicBuilder, Transfer};
    use substrate_test_runtime_client::{TestClientBuilder, TestClientBuilderExt};
    use tokio::sync::Mutex;

    use super::*;

    const SOURCE: TransactionSource = TransactionSource::External;

    // Note:
    // Maximum normal extrinsic size for `substrate_test_runtime` is ~65% of max_block (refer to
    // `substrate_test_runtime::RuntimeBlockWeights` for details).
    // This extrinsic sizing allows for:
    // - one huge xts + a lot of tiny dust
    // - one huge, no medium,
    // - two medium xts
    // This is widely exploited in following tests.
    const HUGE: u32 = 649_000_000;
    const MEDIUM: u32 = 250_000_000;
    const TINY: u32 = 1_000;

    fn extrinsic(nonce: u64) -> Extrinsic {
        ExtrinsicBuilder::new_fill_block(Perbill::from_parts(TINY)).nonce(nonce).build()
    }

    fn chain_event<B: BlockT>(header: B::Header) -> ChainEvent<B>
    where
        NumberFor<B>: From<u64>,
    {
        ChainEvent::NewBestBlock { hash: header.hash(), tree_route: None }
    }

    #[test]
    fn should_cease_building_block_when_deadline_is_reached() {
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());

        block_on(txpool.submit_at(&BlockId::number(0), SOURCE, vec![extrinsic(0), extrinsic(1)])).unwrap();

        block_on(
            txpool.maintain(chain_event(
                client.expect_header(client.info().genesis_hash).expect("there should be header"),
            )),
        );

        let mut proposer_factory = ProposerFactory::new(spawner, client.clone(), txpool.clone(), None);

        let cell = Mutex::new((false, time::Instant::now()));
        let proposer = proposer_factory.init_with_now(
            &client.expect_header(client.info().genesis_hash).unwrap(),
            Box::new(move || {
                let mut value = cell.lock();
                if !value.0 {
                    value.0 = true;
                    return value.1;
                }
                let old = value.1;
                let new = old + time::Duration::from_secs(1);
                *value = (true, new);
                old
            }),
        );

        // when
        let deadline = time::Duration::from_secs(3);
        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, None))
            .map(|r| r.block)
            .unwrap();

        // then
        // block should have some extrinsics although we have some more in the pool.
        assert_eq!(block.extrinsics().len(), 1);
        assert_eq!(txpool.ready().count(), 2);
    }

    #[test]
    fn should_not_panic_when_deadline_is_reached() {
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());

        let mut proposer_factory = ProposerFactory::new(spawner, client.clone(), txpool, None);

        let cell = Mutex::new((false, time::Instant::now()));
        let proposer = proposer_factory.init_with_now(
            &client.expect_header(client.info().genesis_hash).unwrap(),
            Box::new(move || {
                let mut value = cell.lock();
                if !value.0 {
                    value.0 = true;
                    return value.1;
                }
                let new = value.1 + time::Duration::from_secs(160);
                *value = (true, new);
                new
            }),
        );

        let deadline = time::Duration::from_secs(1);

        block_on(proposer.propose(Default::default(), Default::default(), deadline, None)).map(|r| r.block).unwrap();
    }

    #[test]
    fn proposed_storage_changes_should_match_execute_block_storage_changes() {
        let (client, backend) = TestClientBuilder::new().build_with_backend();
        let client = Arc::new(client);
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());

        let genesis_hash = client.info().best_hash;

        block_on(txpool.submit_at(&BlockId::number(0), SOURCE, vec![extrinsic(0)])).unwrap();

        block_on(
            txpool.maintain(chain_event(
                client.expect_header(client.info().genesis_hash).expect("there should be header"),
            )),
        );

        let mut proposer_factory = ProposerFactory::new(spawner, client.clone(), txpool, None);

        let proposer = proposer_factory
            .init_with_now(&client.header(genesis_hash).unwrap().unwrap(), Box::new(time::Instant::now));

        let deadline = time::Duration::from_secs(9);
        let proposal = block_on(proposer.propose(Default::default(), Default::default(), deadline, None)).unwrap();

        assert_eq!(proposal.block.extrinsics().len(), 1);

        let api = client.runtime_api();
        api.execute_block(genesis_hash, proposal.block).unwrap();

        let state = backend.state_at(genesis_hash).unwrap();

        let storage_changes = api.into_storage_changes(&state, genesis_hash).unwrap();

        assert_eq!(proposal.storage_changes.transaction_storage_root, storage_changes.transaction_storage_root,);
    }

    // This test ensures that if one transaction of a user was rejected, because for example
    // the weight limit was hit, we don't mark the other transactions of the user as invalid because
    // the nonce is not matching.
    #[test]
    fn should_not_remove_invalid_transactions_from_the_same_sender_after_one_was_invalid() {
        // given
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());

        let medium = |nonce| ExtrinsicBuilder::new_fill_block(Perbill::from_parts(MEDIUM)).nonce(nonce).build();
        let huge = |nonce| ExtrinsicBuilder::new_fill_block(Perbill::from_parts(HUGE)).nonce(nonce).build();

        block_on(txpool.submit_at(
            &BlockId::number(0),
            SOURCE,
            vec![medium(0), medium(1), huge(2), medium(3), huge(4), medium(5), medium(6)],
        ))
        .unwrap();

        let mut proposer_factory = ProposerFactory::new(spawner, client.clone(), txpool.clone(), None);
        let mut propose_block =
            |client: &TestClient, parent_number, expected_block_extrinsics, expected_pool_transactions| {
                let hash = client.expect_block_hash_from_id(&BlockId::Number(parent_number)).unwrap();
                let proposer =
                    proposer_factory.init_with_now(&client.expect_header(hash).unwrap(), Box::new(time::Instant::now));

                // when
                let deadline = time::Duration::from_secs(900);
                let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, None))
                    .map(|r| r.block)
                    .unwrap();

                // then
                // block should have some extrinsics although we have some more in the pool.
                assert_eq!(txpool.ready().count(), expected_pool_transactions, "at block: {}", block.header.number);
                assert_eq!(block.extrinsics().len(), expected_block_extrinsics, "at block: {}", block.header.number);

                block
            };

        let import_and_maintain = |mut client: Arc<TestClient>, block: TestBlock| {
            let hash = block.hash();
            block_on(client.import(BlockOrigin::Own, block)).unwrap();
            block_on(txpool.maintain(chain_event(client.expect_header(hash).expect("there should be header"))));
        };

        block_on(
            txpool.maintain(chain_event(
                client.expect_header(client.info().genesis_hash).expect("there should be header"),
            )),
        );
        assert_eq!(txpool.ready().count(), 7);

        // let's create one block and import it
        let block = propose_block(&client, 0, 2, 7);
        import_and_maintain(client.clone(), block);
        assert_eq!(txpool.ready().count(), 5);

        // now let's make sure that we can still make some progress
        let block = propose_block(&client, 1, 1, 5);
        import_and_maintain(client.clone(), block);
        assert_eq!(txpool.ready().count(), 4);

        // again let's make sure that we can still make some progress
        let block = propose_block(&client, 2, 1, 4);
        import_and_maintain(client.clone(), block);
        assert_eq!(txpool.ready().count(), 3);

        // again let's make sure that we can still make some progress
        let block = propose_block(&client, 3, 1, 3);
        import_and_maintain(client.clone(), block);
        assert_eq!(txpool.ready().count(), 2);

        // again let's make sure that we can still make some progress
        let block = propose_block(&client, 4, 2, 2);
        import_and_maintain(client.clone(), block);
        assert_eq!(txpool.ready().count(), 0);
    }

    #[test]
    fn should_keep_adding_transactions_after_exhausts_resources_before_soft_deadline() {
        // given
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());

        let tiny = |nonce| ExtrinsicBuilder::new_fill_block(Perbill::from_parts(TINY)).nonce(nonce).build();
        let huge = |who| {
            ExtrinsicBuilder::new_fill_block(Perbill::from_parts(HUGE)).signer(AccountKeyring::numeric(who)).build()
        };

        block_on(txpool.submit_at(
            &BlockId::number(0),
            SOURCE,
            // add 2 * MAX_SKIPPED_TRANSACTIONS that exhaust resources
            (0..MAX_SKIPPED_TRANSACTIONS * 2)
					.map(huge)
					// and some transactions that are okay.
					.chain((0..MAX_SKIPPED_TRANSACTIONS as u64).map(tiny))
					.collect(),
        ))
        .unwrap();

        block_on(
            txpool.maintain(chain_event(
                client.expect_header(client.info().genesis_hash).expect("there should be header"),
            )),
        );
        assert_eq!(txpool.ready().count(), MAX_SKIPPED_TRANSACTIONS * 3);

        let mut proposer_factory = ProposerFactory::new(spawner, client.clone(), txpool, None);

        let cell = Mutex::new(time::Instant::now());
        let proposer = proposer_factory.init_with_now(
            &client.expect_header(client.info().genesis_hash).unwrap(),
            Box::new(move || {
                let mut value = cell.lock();
                let old = *value;
                *value = old + time::Duration::from_secs(1);
                old
            }),
        );

        // when
        // give it enough time so that deadline is never triggered.
        let deadline = time::Duration::from_secs(900);
        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, None))
            .map(|r| r.block)
            .unwrap();

        // then block should have all non-exhaust resources extrinsics (+ the first one).
        assert_eq!(block.extrinsics().len(), MAX_SKIPPED_TRANSACTIONS + 1);
    }

    #[test]
    fn should_only_skip_up_to_some_limit_after_soft_deadline() {
        // given
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());

        let tiny = |who| {
            ExtrinsicBuilder::new_fill_block(Perbill::from_parts(TINY))
                .signer(AccountKeyring::numeric(who))
                .nonce(1)
                .build()
        };
        let huge = |who| {
            ExtrinsicBuilder::new_fill_block(Perbill::from_parts(HUGE)).signer(AccountKeyring::numeric(who)).build()
        };

        block_on(txpool.submit_at(
            &BlockId::number(0),
            SOURCE,
            (0..MAX_SKIPPED_TRANSACTIONS + 2)
					.map(huge)
					// and some transactions that are okay.
					.chain((0..MAX_SKIPPED_TRANSACTIONS + 2).map(tiny))
					.collect(),
        ))
        .unwrap();

        block_on(
            txpool.maintain(chain_event(
                client.expect_header(client.info().genesis_hash).expect("there should be header"),
            )),
        );
        assert_eq!(txpool.ready().count(), MAX_SKIPPED_TRANSACTIONS * 2 + 4);

        let mut proposer_factory = ProposerFactory::new(spawner, client.clone(), txpool, None);

        let deadline = time::Duration::from_secs(600);
        let cell = Arc::new(Mutex::new((0, time::Instant::now())));
        let cell2 = cell.clone();
        let proposer = proposer_factory.init_with_now(
            &client.expect_header(client.info().genesis_hash).unwrap(),
            Box::new(move || {
                let mut value = cell.lock();
                let (called, old) = *value;
                // add time after deadline is calculated internally (hence 1)
                let increase = if called == 1 {
                    // we start after the soft_deadline should have already been reached.
                    deadline / 2
                } else {
                    // but we make sure to never reach the actual deadline
                    time::Duration::from_millis(0)
                };
                *value = (called + 1, old + increase);
                old
            }),
        );

        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, None))
            .map(|r| r.block)
            .unwrap();

        // then the block should have one or two transactions. This maybe random as they are
        // processed in parallel. The same signer and consecutive nonces for huge and tiny
        // transactions guarantees that max two transactions will get to the block.
        assert!((1..3).contains(&block.extrinsics().len()), "Block shall contain one or two extrinsics.");
        assert!(
            cell2.lock().0 > MAX_SKIPPED_TRANSACTIONS,
            "Not enough calls to current time, which indicates the test might have ended because of deadline, not \
             soft deadline"
        );
    }

    #[test]
    fn should_cease_building_block_when_block_limit_is_reached() {
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());
        let genesis_header = client.expect_header(client.info().genesis_hash).expect("there should be header");

        let extrinsics_num = 5;
        let extrinsics = std::iter::once(
            Transfer { from: AccountKeyring::Alice.into(), to: AccountKeyring::Bob.into(), amount: 100, nonce: 0 }
                .into_unchecked_extrinsic(),
        )
        .chain((1..extrinsics_num as u64).map(extrinsic))
        .collect::<Vec<_>>();

        let block_limit = genesis_header.encoded_size()
            + extrinsics.iter().take(extrinsics_num - 1).map(Encode::encoded_size).sum::<usize>()
            + Vec::<Extrinsic>::new().encoded_size();

        block_on(txpool.submit_at(&BlockId::number(0), SOURCE, extrinsics)).unwrap();

        block_on(txpool.maintain(chain_event(genesis_header.clone())));

        let mut proposer_factory = ProposerFactory::new(spawner, client, txpool, None);

        let proposer = block_on(proposer_factory.init(&genesis_header)).unwrap();

        // Give it enough time
        let deadline = time::Duration::from_secs(300);
        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, Some(block_limit)))
            .map(|r| r.block)
            .unwrap();

        // Based on the block limit, one transaction shouldn't be included.
        assert_eq!(block.extrinsics().len(), extrinsics_num - 1);

        let proposer = block_on(proposer_factory.init(&genesis_header)).unwrap();

        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, None))
            .map(|r| r.block)
            .unwrap();

        // Without a block limit it should include all of them
        assert_eq!(block.extrinsics().len(), extrinsics_num);
    }

    #[test]
    fn proposer_factory_can_update_default_block_size_limit() {
        let client = Arc::new(substrate_test_runtime_client::new());
        let spawner = sp_core::testing::TaskExecutor::new();
        let txpool = BasicPool::new_full(Default::default(), true.into(), None, spawner.clone(), client.clone());
        let genesis_header = client.expect_header(client.info().genesis_hash).expect("there should be header");

        let extrinsics_num = 5;
        let extrinsics = std::iter::once(
            Transfer { from: AccountKeyring::Alice.into(), to: AccountKeyring::Bob.into(), amount: 100, nonce: 0 }
                .into_unchecked_extrinsic(),
        )
        .chain((1..extrinsics_num as u64).map(extrinsic))
        .collect::<Vec<_>>();

        let block_limit = genesis_header.encoded_size()
            + extrinsics.iter().take(extrinsics_num - 1).map(Encode::encoded_size).sum::<usize>()
            + Vec::<Extrinsic>::new().encoded_size();

        block_on(txpool.submit_at(&BlockId::number(0), SOURCE, extrinsics)).unwrap();

        block_on(txpool.maintain(chain_event(genesis_header.clone())));

        let mut proposer_factory = ProposerFactory::new(spawner, client, txpool, None);
        proposer_factory.set_default_block_size_limit(block_limit);

        let proposer = block_on(proposer_factory.init(&genesis_header)).unwrap();

        // Give it enough time
        let deadline = time::Duration::from_secs(300);
        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, Default::default()))
            .map(|r| r.block)
            .unwrap();

        // Based on the block limit, one transaction shouldn't be included.
        assert_eq!(block.extrinsics().len(), extrinsics_num - 1);

        // increase block size limit
        proposer_factory.set_default_block_size_limit(block_limit * 2);
        let proposer = block_on(proposer_factory.init(&genesis_header)).unwrap();

        let block = block_on(proposer.propose(Default::default(), Default::default(), deadline, None))
            .map(|r| r.block)
            .unwrap();

        // with increased blocklimit we should include all of them
        assert_eq!(block.extrinsics().len(), extrinsics_num);
    }
}

// async fn submit_to_da(data: &str) -> String {
// dotenv().ok();
// let da_host = env::var("DA_HOST").expect("DA_HOST must be set");
// let da_namespace = env::var("DA_NAMESPACE").expect("DA_NAMESPACE must be set");
// let da_auth_token = env::var("DA_AUTH_TOKEN").expect("DA_AUTH_TOKEN must be set");
// let da_auth = format!("Bearer {}", da_auth_token);
//
// println!("this is the da_namespace: {:?}", da_namespace);
//
// let client = Client::new();
// let rpc_request = json!({
// "jsonrpc": "2.0",
// "method": "blob.Submit",
// "params": [
// [
// {
// "namespace": da_namespace,
// "data": data,
// }
// ]
// ],
// "id": 1,
// });
//
// let uri = std::env::var("da_uri").unwrap_or(da_host.into());
//
// Token should be removed from code.
// let req = Request::post(uri.as_str())
// .header(AUTHORIZATION, HeaderValue::from_str(da_auth.as_str()).unwrap())
// .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
// .body(Body::from(rpc_request.to_string()))
// .unwrap();
// let response_future = client.request(req);
//
// let resp = tokio::time::timeout(Duration::from_secs(100), response_future)
// .await
// .map_err(|_| "Request timed out")
// .unwrap()
// .unwrap();
//
// let response_body = hyper::body::to_bytes(resp.into_body()).await.unwrap();
// let parsed: Value = serde_json::from_slice(&response_body).unwrap();
//
// if let Some(result_value) = parsed.get("result") { result_value.to_string() } else {
// "".to_string() } }
//
// fn encode_data_to_base64(original: &str) -> String {
// Convert string to bytes
// let bytes = original.as_bytes();
// Convert bytes to base64
// let base64_str: String = general_purpose::STANDARD.encode(&bytes);
// base64_str
// }
