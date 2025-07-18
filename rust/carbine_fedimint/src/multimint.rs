use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Display},
    str::FromStr,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use anyhow::bail;
use anyhow::Context;
use bitcoin::key::rand::{seq::SliceRandom, thread_rng};
use fedimint_api_client::api::net::Connector;
use fedimint_bip39::{Bip39RootSecretStrategy, Language, Mnemonic};
use fedimint_client::{
    db::ChronologicalOperationLogKey,
    module::{module::recovery::RecoveryProgress, oplog::OperationLogEntry},
    module_init::ClientModuleInitRegistry,
    secret::RootSecretStrategy,
    Client, ClientHandleArc, OperationId,
};
use fedimint_core::{
    config::{ClientConfig, FederationId},
    db::{mem_impl::MemDatabase, Database, IDatabaseTransactionOpsCoreTyped},
    encoding::{Decodable, Encodable},
    hex,
    invite_code::InviteCode,
    task::TaskGroup,
    util::SafeUrl,
    Amount,
};
use fedimint_derive_secret::{ChildId, DerivableSecret};
use fedimint_ln_client::{
    InternalPayState, LightningClientInit, LightningClientModule, LightningOperationMetaPay,
    LightningOperationMetaVariant, LnPayState, LnReceiveState,
};
use fedimint_ln_common::LightningGateway;
use fedimint_lnv2_client::{
    FinalReceiveOperationState, LightningOperationMeta, ReceiveOperationState, SendOperationState,
};
use fedimint_lnv2_common::{gateway_api::PaymentFee, Bolt11InvoiceDescription};
use fedimint_meta_client::{common::DEFAULT_META_KEY, MetaClientInit};
use fedimint_mint_client::{
    api::MintFederationApi, MintClientInit, MintClientModule, MintOperationMeta,
    MintOperationMetaVariant, OOBNotes, ReissueExternalNotesState, SelectNotesWithAtleastAmount,
    SpendOOBState,
};
use fedimint_wallet_client::client_db::TweakIdx;
use fedimint_wallet_client::WithdrawState;
use fedimint_wallet_client::{api::WalletFederationApi, TxOutputSummary};
use fedimint_wallet_client::{
    DepositStateV2, PegOutFees, WalletClientInit, WalletClientModule, WalletOperationMeta,
    WalletOperationMetaVariant,
};
use futures_util::StreamExt;
use lightning_invoice::{Bolt11Invoice, Description};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, json};
use tokio::sync::RwLock;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::Instant,
};

use crate::{
    anyhow,
    db::{
        BtcPrice, BtcPriceKey, DisplaySetting, DisplaySettingKey, FederationMetaKey,
        LightningAddressConfig, LightningAddressKey, LightningAddressKeyPrefix,
    },
    error_to_flutter, info_to_flutter, FederationConfig, FederationConfigKey,
    FederationConfigKeyPrefix, SeedPhraseAckKey,
};
use crate::{event_bus::EventBus, get_event_bus};

const DEFAULT_EXPIRY_TIME_SECS: u32 = 86400;
const CACHE_UPDATE_INTERVAL_SECS: u64 = 60 * 10;

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct PaymentPreview {
    pub amount_msats: u64,
    pub payment_hash: String,
    pub network: String,
    pub invoice: String,
    pub gateway: String,
    pub amount_with_fees: u64,
    pub is_lnv2: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug, Encodable, Decodable)]
pub struct FederationSelector {
    pub federation_name: String,
    pub federation_id: FederationId,
    pub network: Option<String>,
}

impl Display for FederationSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.federation_name)
    }
}

#[derive(Clone, PartialEq, Serialize, Debug)]
pub struct WithdrawFeesResponse {
    pub fee_amount: u64,
    pub fee_rate_sats_per_vb: f64,
    pub tx_size_vbytes: u32,
    pub peg_out_fees: PegOutFees,
}

#[derive(Clone)]
pub struct Multimint {
    db: Database,
    mnemonic: Mnemonic,
    modules: ClientModuleInitRegistry,
    clients: Arc<RwLock<BTreeMap<FederationId, ClientHandleArc>>>,
    task_group: TaskGroup,
    pegin_address_monitor_tx: UnboundedSender<(FederationId, TweakIdx)>,
    recovery_progress: Arc<RwLock<BTreeMap<FederationId, BTreeMap<u16, RecoveryProgress>>>>,
    internal_ecash_spends: Arc<RwLock<BTreeSet<OperationId>>>,
    allocated_bitcoin_addresses:
        Arc<RwLock<BTreeMap<FederationId, BTreeMap<TweakIdx, (String, Option<u64>)>>>>,
    recurringd_invoices: Arc<RwLock<BTreeSet<OperationId>>>,
}

#[derive(Debug, Serialize, Encodable, Decodable, Clone)]
pub struct FederationMeta {
    pub picture: Option<String>,
    pub welcome: Option<String>,
    pub guardians: Vec<Guardian>,
    pub selector: FederationSelector,
    pub last_updated: u64,
}

#[derive(Debug, Serialize, Clone, Eq, PartialEq, Encodable, Decodable)]
pub struct Guardian {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Transaction {
    pub kind: TransactionKind,
    pub amount: u64,
    pub timestamp: u64,
    pub operation_id: Vec<u8>,
}

#[derive(Debug, Serialize, Clone)]
pub enum TransactionKind {
    LightningReceive {
        fees: u64,
        gateway: String,
        payee_pubkey: String,
        payment_hash: String,
    },
    LightningSend {
        fees: u64,
        gateway: String,
        payment_hash: String,
        preimage: String,
    },
    LightningRecurring,
    OnchainReceive,
    OnchainSend,
    EcashReceive {
        oob_notes: String,
        fees: u64,
    },
    EcashSend {
        oob_notes: String,
        fees: u64,
    },
}

#[derive(Debug, Serialize, Clone, Eq, PartialEq)]
pub struct Utxo {
    pub txid: String,
    pub index: u32,
    pub amount: u64,
}

impl From<TxOutputSummary> for Utxo {
    fn from(value: TxOutputSummary) -> Self {
        Self {
            txid: value.outpoint.txid.to_string(),
            index: value.outpoint.vout,
            amount: value.amount.to_sat() * 1000,
        }
    }
}

pub enum MultimintCreation {
    New,
    LoadExisting,
    NewFromMnemonic { words: Vec<String> },
}

#[derive(Debug, Eq, PartialEq)]
pub enum ClientType {
    New,
    Temporary,
    Recovery { client_config: ClientConfig },
}

impl fmt::Display for ClientType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientType::New => write!(f, "New"),
            ClientType::Temporary => write!(f, "Temporary"),
            ClientType::Recovery { .. } => write!(f, "Recovery"),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct MempoolEvent {
    pub amount: u64,
    pub txid: String,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct AwaitingConfsEvent {
    pub amount: u64,
    pub txid: String,
    pub block_height: u64,
    pub needed: u64,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct ConfirmedEvent {
    pub amount: u64,
    pub txid: String,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct ClaimedEvent {
    pub amount: u64,
    pub txid: String,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub enum DepositEventKind {
    Mempool(MempoolEvent),
    AwaitingConfs(AwaitingConfsEvent),
    Confirmed(ConfirmedEvent),
    Claimed(ClaimedEvent),
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct InvoicePaidEvent {
    pub amount_msats: u64,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub enum LightningEventKind {
    InvoicePaid(InvoicePaidEvent),
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub enum MultimintEvent {
    Deposit((FederationId, DepositEventKind)),
    Lightning((FederationId, LightningEventKind)),
    Log(LogLevel, String),
    RecoveryDone(String),
    RecoveryProgress(String, u16, u32, u32),
    Ecash((FederationId, u64)),
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub enum LightningSendOutcome {
    Success(String),
    Failure,
}

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct FedimintGateway {
    pub endpoint: String,
    pub base_routing_fee: u64,
    pub ppm_routing_fee: u64,
    pub base_transaction_fee: u64,
    pub ppm_transaction_fee: u64,
    pub lightning_alias: Option<String>,
    pub lightning_node: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LNAddressRegisterRequest {
    pub domain: String,
    pub username: String,
    pub lnurl: String,
}

#[derive(Debug, Clone, Serialize)]
pub enum LNAddressStatus {
    Registered { lnurl: String },
    Available,
    CurrentConfig,
    UnsupportedFederation,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct LNAddressRemoveRequest {
    pub domain: String,
    pub username: String,
    pub authentication_token: String,
}

impl Multimint {
    pub async fn new(db: Database, creation_type: MultimintCreation) -> anyhow::Result<Self> {
        let start = Instant::now();
        let mnemonic = match creation_type {
            MultimintCreation::New => {
                let mnemonic = Bip39RootSecretStrategy::<12>::random(&mut thread_rng());
                Client::store_encodable_client_secret(&db, mnemonic.to_entropy()).await?;
                info_to_flutter("Created new multimint wallet").await;
                mnemonic
            }
            MultimintCreation::LoadExisting => {
                let entropy = Client::load_decodable_client_secret::<Vec<u8>>(&db)
                    .await
                    .expect("Could not load existing secret");
                let mnemonic = Mnemonic::from_entropy(&entropy)?;
                info_to_flutter("Loaded existing multimint wallet").await;
                mnemonic
            }
            MultimintCreation::NewFromMnemonic { words } => {
                let all_words = words.join(" ");
                let mnemonic =
                    Mnemonic::parse_in_normalized(Language::English, all_words.as_str())?;
                Client::store_encodable_client_secret(&db, mnemonic.to_entropy()).await?;
                info_to_flutter("Created new multimint wallet from mnemonic").await;
                mnemonic
            }
        };

        let mut modules = ClientModuleInitRegistry::new();
        modules.attach(LightningClientInit::default());
        modules.attach(MintClientInit);
        modules.attach(WalletClientInit::default());
        modules.attach(fedimint_lnv2_client::LightningClientInit::default());
        modules.attach(MetaClientInit);

        let clients = Arc::new(RwLock::new(BTreeMap::new()));

        let (pegin_address_monitor_tx, pegin_address_monitor_rx) =
            unbounded_channel::<(FederationId, TweakIdx)>();

        let mut multimint = Self {
            db,
            mnemonic,
            modules,
            clients: clients.clone(),
            task_group: TaskGroup::new(),
            pegin_address_monitor_tx: pegin_address_monitor_tx.clone(),
            recovery_progress: Arc::new(RwLock::new(BTreeMap::new())),
            internal_ecash_spends: Arc::new(RwLock::new(BTreeSet::new())),
            allocated_bitcoin_addresses: Arc::new(RwLock::new(BTreeMap::new())),
            recurringd_invoices: Arc::new(RwLock::new(BTreeSet::new())),
        };

        multimint.load_clients().await?;
        multimint
            .spawn_pegin_address_watcher(pegin_address_monitor_rx)
            .await?;
        multimint.monitor_all_unused_pegin_addresses().await?;
        multimint.spawn_cache_task();
        multimint.spawn_recurring_invoice_listener();

        info_to_flutter(format!("Initialized Multimint in {:?}", start.elapsed())).await;
        Ok(multimint)
    }

    async fn load_clients(&mut self) -> anyhow::Result<()> {
        info_to_flutter("Loading all clients...").await;
        let mut dbtx = self.db.begin_transaction_nc().await;
        let configs = dbtx
            .find_by_prefix(&FederationConfigKeyPrefix)
            .await
            .collect::<BTreeMap<FederationConfigKey, FederationConfig>>()
            .await;
        for (id, _) in configs {
            let client_db = self.get_client_database(&id.id);
            let mut client_builder = Client::builder(client_db).await?;
            client_builder.with_module_inits(self.modules.clone());
            client_builder.with_primary_module_kind(fedimint_mint_client::KIND);
            let secret = Self::derive_federation_secret(&self.mnemonic, &id.id);
            let client = client_builder.open(secret).await.map(Arc::new)?;

            self.clients.write().await.insert(id.id, client.clone());

            self.finish_active_subscriptions(client.clone(), id.id)
                .await;
            if client.has_pending_recoveries() {
                self.spawn_recovery_progress(client.clone());
            }
        }

        Ok(())
    }

    async fn finish_active_subscriptions(
        &self,
        client: ClientHandleArc,
        federation_id: FederationId,
    ) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("finish active subscriptions", async move {
                let active_operations = client.get_active_operations().await;
                let operation_log = client.operation_log();
                for op_id in active_operations {
                    let entry = operation_log.get_operation(op_id).await;
                    if let Some(entry) = entry {
                        match entry.operation_module_kind() {
                            "lnv2" | "ln" => {
                                // We could check what type of operation this is, but `await_receive` and `await_send`
                                // will do that internally. So we just spawn both here and let one fail since it is the wrong
                                // operation type.
                                self_copy.spawn_await_receive(federation_id, op_id);
                                self_copy.spawn_await_send(federation_id, op_id);
                            }
                            "mint" => {
                                // We could check what type of operation this is, but `await_ecash_reissue` and `await_ecash_send`
                                // will do that internally. So we just spawn both here and let one fail since it is the wrong
                                // operation type.
                                self_copy.spawn_await_ecash_reissue(federation_id, op_id);
                                self_copy.spawn_await_ecash_send(federation_id, op_id);
                            }
                            // Wallet operations are handled by the pegin monitor
                            "wallet" => {}
                            module => {
                                info_to_flutter(format!(
                                    "Active operation needs to be driven to completion: {module}"
                                ))
                                .await;
                            }
                        }
                    }
                }
            });
    }

    async fn spawn_pegin_address_watcher(
        &self,
        mut monitor_rx: UnboundedReceiver<(FederationId, TweakIdx)>,
    ) -> anyhow::Result<()> {
        let event_bus_clone = get_event_bus();
        let task_group_clone = self.task_group.clone();
        let clients_clone = self.clients.clone();
        let addresses_clone = self.allocated_bitcoin_addresses.clone();

        self.task_group
            .spawn_cancellable("pegin address watcher", async move {
                while let Some((fed_id, tweak_idx)) = monitor_rx.recv().await {
                    let event_bus = event_bus_clone.clone();
                    // wrapping the clients in Arc<RwLock<..>> allows us to monitor using clients
                    // created after the background task is spawned
                    let client = clients_clone
                        .read()
                        .await
                        .get(&fed_id)
                        .expect("No federation exists")
                        .clone();

                    let addresses_clone = addresses_clone.clone();
                    task_group_clone.spawn_cancellable("tweak index watcher", async move {
                        if let Err(e) = Self::watch_pegin_address(
                            fed_id,
                            client,
                            tweak_idx,
                            event_bus,
                            addresses_clone,
                        )
                        .await
                        {
                            info_to_flutter(format!(
                                "watch_pegin_address({}) failed: {:?}",
                                tweak_idx.0, e
                            ))
                            .await;
                        }
                    });
                }
            });

        Ok(())
    }

    async fn watch_pegin_address(
        federation_id: FederationId,
        client: ClientHandleArc,
        tweak_idx: TweakIdx,
        event_bus: EventBus<MultimintEvent>,
        addresses: Arc<RwLock<BTreeMap<FederationId, BTreeMap<TweakIdx, (String, Option<u64>)>>>>,
    ) -> anyhow::Result<()> {
        let wallet_module = client.get_first_module::<WalletClientModule>()?;

        let data = match wallet_module.get_pegin_tweak_idx(tweak_idx).await {
            Ok(d) => d,
            Err(e) if e.to_string().contains("TweakIdx not found") => return Ok(()),
            Err(e) => return Err(e),
        };

        let mut updates = wallet_module
            .subscribe_deposit(data.operation_id)
            .await?
            .into_stream();

        while let Some(state) = updates.next().await {
            match state {
                DepositStateV2::WaitingForTransaction => {}
                DepositStateV2::WaitingForConfirmation {
                    btc_deposited,
                    btc_out_point,
                } => {
                    let deposit_event = MultimintEvent::Deposit((
                        federation_id,
                        DepositEventKind::Mempool(MempoolEvent {
                            amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                            txid: btc_out_point.txid.to_string(),
                        }),
                    ));

                    event_bus.publish(deposit_event).await;

                    let client = reqwest::Client::new();

                    let api_url = match wallet_module.get_network() {
                        bitcoin::Network::Bitcoin => "https://mempool.space/api".to_string(),
                        bitcoin::Network::Signet => "https://mutinynet.com/api".to_string(),
                        bitcoin::Network::Regtest => {
                            // referencing devimint, uncomment for regtest
                            // "http://localhost:{FM_PORT_ESPLORA}".to_string()
                            panic!("Regtest requires manually setting the connection params")
                        }
                        network => {
                            panic!("{network} is not a supported network")
                        }
                    };

                    let tx_height = fedimint_core::util::retry(
                        "get confirmed block height",
                        fedimint_core::util::backoff_util::background_backoff(),
                        || async {
                            let resp = client
                                .get(format!("{}/tx/{}", api_url, btc_out_point.txid.to_string(),))
                                .send()
                                .await?
                                .error_for_status()?
                                .text()
                                .await?;

                            serde_json::from_str::<serde_json::Value>(&resp)?
                                .get("status")
                                .and_then(|s| s.get("block_height"))
                                .and_then(|h| h.as_u64())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("no confirmation height yet, still in mempool")
                                })
                        },
                    )
                    .await
                    .expect("Never gives up");

                    let every_10_secs = fedimint_core::util::backoff_util::custom_backoff(
                        Duration::from_secs(10),
                        Duration::from_secs(10),
                        None,
                    );
                    fedimint_core::util::retry("consensus confirmation", every_10_secs, || async {
                        let consensus_height = wallet_module
                            .api
                            .fetch_consensus_block_count()
                            .await?
                            .saturating_sub(1);

                        let needed = tx_height.saturating_sub(consensus_height);

                        let deposit_event = MultimintEvent::Deposit((
                            federation_id,
                            DepositEventKind::AwaitingConfs(AwaitingConfsEvent {
                                amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                                txid: btc_out_point.txid.to_string(),
                                block_height: tx_height,
                                needed,
                            }),
                        ));

                        event_bus.publish(deposit_event).await;
                        anyhow::ensure!(needed == 0, "{} more confs needed", needed);

                        Ok(())
                    })
                    .await
                    .expect("Never gives up");

                    // trigger another check of pegin monitor for faster claim
                    wallet_module.recheck_pegin_address(tweak_idx).await?;
                }
                DepositStateV2::Confirmed {
                    btc_deposited,
                    btc_out_point,
                } => {
                    let mut addresses = addresses.write().await;
                    if let Some(fed_addresses) = addresses.get_mut(&federation_id) {
                        if let Some((address, _)) = fed_addresses.remove(&tweak_idx) {
                            fed_addresses
                                .insert(tweak_idx, (address, Some(btc_deposited.to_sat())));
                        }
                    }

                    let deposit_event = MultimintEvent::Deposit((
                        federation_id,
                        DepositEventKind::Confirmed(ConfirmedEvent {
                            amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                            txid: btc_out_point.txid.to_string(),
                        }),
                    ));

                    event_bus.publish(deposit_event).await;
                }
                DepositStateV2::Claimed {
                    btc_deposited,
                    btc_out_point,
                } => {
                    let deposit_event = MultimintEvent::Deposit((
                        federation_id,
                        DepositEventKind::Claimed(ClaimedEvent {
                            amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                            txid: btc_out_point.txid.to_string(),
                        }),
                    ));

                    event_bus.publish(deposit_event).await;
                }
                DepositStateV2::Failed(e) => {
                    info_to_flutter(format!("deposit failed: {:?}", e)).await;
                    break;
                }
            };
        }

        Ok(())
    }

    async fn monitor_all_unused_pegin_addresses(&self) -> anyhow::Result<()> {
        let federation_ids = self
            .federations()
            .await
            .into_iter()
            .map(|(fed, _)| fed.federation_id);
        let pegin_address_monitor_tx_clone = self.pegin_address_monitor_tx.clone();
        let clients_clone = self.clients.clone();
        let addresses_clone = self.allocated_bitcoin_addresses.clone();

        self.task_group
            .spawn_cancellable("unused address monitor", async move {
                for fed_id in federation_ids {
                    let client = clients_clone
                        .read()
                        .await
                        .get(&fed_id)
                        .expect("No federation exists")
                        .clone();
                    let wallet_module = client
                        .get_first_module::<WalletClientModule>()
                        .expect("No wallet module exists");

                    let operation_log = client.operation_log();

                    let mut tweak_idx = TweakIdx(0);
                    while let Ok(data) = wallet_module.get_pegin_tweak_idx(tweak_idx).await {
                        if data.claimed.is_empty() {
                            // we found an allocated, unused address so we need to monitor
                            if let Err(_) = pegin_address_monitor_tx_clone.send((fed_id, tweak_idx))
                            {
                                info_to_flutter(format!(
                                    "failed to monitor tweak index {:?} for fed {:?}",
                                    tweak_idx, fed_id
                                ))
                                .await;
                            }
                        }
                        tweak_idx = tweak_idx.next();

                        let operation = operation_log.get_operation(data.operation_id).await;
                        if let Some(wallet_op) = operation {
                            let wallet_meta = wallet_op.meta::<WalletOperationMeta>();
                            if let WalletOperationMetaVariant::Deposit {
                                address,
                                tweak_idx,
                                expires_at: _,
                            } = wallet_meta.variant
                            {
                                let mut addresses = addresses_clone.write().await;
                                let fed_addresses =
                                    addresses.entry(fed_id).or_insert(BTreeMap::new());
                                if let Some(DepositStateV2::Claimed { btc_deposited, .. }) =
                                    wallet_op.outcome()
                                {
                                    fed_addresses.insert(
                                        tweak_idx.expect("Tweak cannot be None"),
                                        (
                                            address.assume_checked().to_string(),
                                            Some(btc_deposited.to_sat()),
                                        ),
                                    );
                                } else {
                                    fed_addresses.insert(
                                        tweak_idx.expect("Tweak cannot be None"),
                                        (address.assume_checked().to_string(), None),
                                    );
                                }
                            }
                        }
                    }
                }
            });

        Ok(())
    }

    pub async fn contains_client(&self, federation_id: &FederationId) -> bool {
        self.clients.read().await.contains_key(federation_id)
    }

    pub async fn has_seed_phrase_ack(&self) -> bool {
        let mut dbtx = self.db.begin_transaction_nc().await;
        dbtx.get_value(&SeedPhraseAckKey).await.is_some()
    }

    pub async fn ack_seed_phrase(&self) {
        let mut dbtx = self.db.begin_transaction().await;
        dbtx.insert_entry(&SeedPhraseAckKey, &()).await;
        dbtx.commit_tx().await;
    }

    async fn get_or_build_temp_client(
        &self,
        invite_code: InviteCode,
    ) -> anyhow::Result<(ClientHandleArc, FederationId)> {
        // Sometimes we want to get the federation meta before we've joined (i.e to show a preview).
        // In this case, we create a temprorary client and retrieve all the data
        let federation_id = invite_code.federation_id();
        let maybe_client = self.clients.read().await.get(&federation_id).cloned();
        let client = if let Some(client) = maybe_client {
            if !client.has_pending_recoveries() {
                client
            } else {
                self.build_client(
                    &federation_id,
                    &invite_code,
                    Connector::Tcp,
                    ClientType::Temporary,
                )
                .await?
            }
        } else {
            self.build_client(
                &federation_id,
                &invite_code,
                Connector::Tcp,
                ClientType::Temporary,
            )
            .await?
        };

        Ok((client, federation_id))
    }

    fn spawn_cache_task(&self) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("cache update", async move {
                // Every 5 seconds this thread will wake up to check if the cached federation meta or the cached bitcoin price
                // needs updating
                let mut interval = tokio::time::interval(Duration::from_secs(5));
                interval.tick().await;
                loop {
                    let now = std::time::SystemTime::now();
                    let threshold = now
                        .checked_sub(Duration::from_secs(CACHE_UPDATE_INTERVAL_SECS))
                        .expect("Cannot be negative");

                    // First check if the federation meta needs updating
                    let mut dbtx = self_copy.db.begin_transaction_nc().await;
                    let configs = dbtx
                        .find_by_prefix(&FederationConfigKeyPrefix)
                        .await
                        .collect::<Vec<_>>()
                        .await;
                    for (key, _) in configs {
                        let federation_id = key.id;

                        let cached_meta =
                            dbtx.get_value(&FederationMetaKey { federation_id }).await;
                        if let Some(cached_meta) = cached_meta {
                            let last_updated =
                                UNIX_EPOCH + Duration::from_millis(cached_meta.last_updated);
                            // Skip over caching this federation's meta if we cached it recently
                            if last_updated >= threshold {
                                continue;
                            }
                        }

                        if let Some(client) = self_copy.clients.read().await.get(&federation_id) {
                            if !client.has_pending_recoveries() {
                                if let Err(e) =
                                    self_copy.cache_federation_meta(client.clone(), now).await
                                {
                                    error_to_flutter(format!(
                                        "Could not cache federation meta {e:?}"
                                    ))
                                    .await;
                                }
                            }
                        }
                    }

                    // Next check if the bitcoin price needs updating. Only update the price if it has not been cached yet, or if
                    // it is out of date
                    let cached_price = dbtx.get_value(&BtcPriceKey).await;
                    if let Some(cached_price) = cached_price {
                        if cached_price.last_updated < threshold {
                            self_copy.cache_btc_price(now).await;
                        }
                    } else {
                        self_copy.cache_btc_price(now).await;
                    }

                    interval.tick().await;
                }
            });
    }

    async fn cache_btc_price(&self, now: std::time::SystemTime) {
        let url = "https://mempool.space/api/v1/prices";
        let Ok(response) = reqwest::get(url).await else {
            error_to_flutter("BTC Price GET returned error").await;
            return;
        };

        if response.status().is_success() {
            let json: Result<serde_json::Value, reqwest::Error> = response.json().await;
            if let Ok(json) = json {
                if let Some(price) = json.get("USD").and_then(|v| v.as_u64()) {
                    let mut dbtx = self.db.begin_transaction().await;
                    dbtx.insert_entry(
                        &BtcPriceKey,
                        &BtcPrice {
                            price,
                            last_updated: now,
                        },
                    )
                    .await;
                    dbtx.commit_tx().await;
                    info_to_flutter(format!("Updated BTC Price: {}", price)).await;
                } else {
                    error_to_flutter("USD price not found in response").await;
                }
            }
        } else {
            error_to_flutter(format!(
                "Failed to load price data, status: {}",
                response.status()
            ))
            .await;
        }
    }

    pub async fn get_cached_federation_meta(
        &self,
        invite: Option<String>,
        federation_id: Option<FederationId>,
    ) -> anyhow::Result<FederationMeta> {
        let (client, federation_id) = match federation_id {
            Some(federation_id) => {
                let clients = self.clients.read().await;
                let client = clients
                    .get(&federation_id)
                    .ok_or(anyhow!("No federation exists"))?
                    .clone();
                (client, federation_id)
            }
            None => {
                let invite =
                    invite.ok_or(anyhow!("Federation ID and Invite cannot both be None"))?;
                let invite_code = InviteCode::from_str(&invite)?;
                self.get_or_build_temp_client(invite_code).await?
            }
        };

        let mut dbtx = self.db.begin_transaction().await;
        if let Some(cached_meta) = dbtx.get_value(&FederationMetaKey { federation_id }).await {
            return Ok(cached_meta);
        }

        // Federation either has not been cached yet, or is a new federation
        self.cache_federation_meta(client, std::time::SystemTime::now())
            .await
    }

    async fn cache_federation_meta(
        &self,
        client: ClientHandleArc,
        now: std::time::SystemTime,
    ) -> anyhow::Result<FederationMeta> {
        let federation_id = client.federation_id();

        let config = client.config().await;
        let wallet = client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;
        let network = wallet.get_network().to_string();

        let peers = &config.global.api_endpoints;
        let mut guardians = Vec::new();
        for (peer_id, endpoint) in peers {
            let fedimintd_vesion = client.api().fedimintd_version(*peer_id).await.ok();
            guardians.push(Guardian {
                name: endpoint.name.clone(),
                version: fedimintd_vesion,
            });
        }

        let selector = FederationSelector {
            federation_name: config.global.federation_name().unwrap_or("").to_string(),
            federation_id,
            network: Some(network),
        };

        let meta = client.get_first_module::<fedimint_meta_client::MetaClientModule>();
        let federation_meta = if let Ok(meta) = meta {
            let consensus = meta.get_consensus_value(DEFAULT_META_KEY).await?;
            match consensus {
                Some(value) => {
                    let val = serde_json::to_value(value).expect("cant fail");
                    let val = val
                        .get("value")
                        .ok_or(anyhow!("value not present"))?
                        .as_str()
                        .ok_or(anyhow!("value was not a string"))?;
                    let str = hex::decode(val)?;
                    let json = String::from_utf8(str)?;
                    let meta: serde_json::Value = serde_json::from_str(&json)?;
                    let welcome = if let Some(welcome) = meta.get("welcome_message") {
                        welcome.as_str().map(|s| s.to_string())
                    } else {
                        None
                    };
                    let picture = if let Some(picture) = meta.get("fedi:federation_icon_url") {
                        let url_str = picture
                            .as_str()
                            .ok_or(anyhow!("icon url is not a string"))?;
                        // Verify that it is a url
                        Some(SafeUrl::parse(url_str)?.to_string())
                    } else {
                        None
                    };

                    FederationMeta {
                        picture,
                        welcome,
                        guardians,
                        selector,
                        last_updated: now
                            .duration_since(UNIX_EPOCH)
                            .expect("Cannot be before epoch")
                            .as_millis() as u64,
                    }
                }
                None => FederationMeta {
                    picture: None,
                    welcome: None,
                    guardians,
                    selector,
                    last_updated: now
                        .duration_since(UNIX_EPOCH)
                        .expect("Cannot be before epoch")
                        .as_millis() as u64,
                },
            }
        } else {
            FederationMeta {
                picture: None,
                welcome: None,
                guardians,
                selector,
                last_updated: now
                    .duration_since(UNIX_EPOCH)
                    .expect("Cannot be before epoch")
                    .as_millis() as u64,
            }
        };

        let mut dbtx = self.db.begin_transaction().await;
        dbtx.insert_entry(&FederationMetaKey { federation_id }, &federation_meta)
            .await;
        dbtx.commit_tx().await;
        info_to_flutter(format!("Updated meta for {federation_id}")).await;

        Ok(federation_meta)
    }

    pub fn get_mnemonic(&self) -> Vec<String> {
        self.mnemonic
            .words()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    }

    pub async fn join_federation(
        &mut self,
        invite: String,
        recover: bool,
    ) -> anyhow::Result<FederationSelector> {
        let invite_code = InviteCode::from_str(&invite)?;
        let federation_id = invite_code.federation_id();
        let client_config = Connector::default()
            .download_from_invite_code(&invite_code)
            .await?;

        let client = if recover {
            self.build_client(
                &federation_id,
                &invite_code,
                Connector::Tcp,
                ClientType::Recovery {
                    client_config: client_config.clone(),
                },
            )
            .await?
        } else {
            self.build_client(
                &federation_id,
                &invite_code,
                Connector::Tcp,
                ClientType::New,
            )
            .await?
        };

        let federation_name = client_config
            .global
            .federation_name()
            .expect("No federation name")
            .to_owned();

        let network = if let Ok(wallet) =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()
        {
            Some(wallet.get_network().to_string())
        } else {
            None
        };

        let federation_config = FederationConfig {
            connector: Connector::default(),
            federation_name: federation_name.clone(),
            network: network.clone(),
            client_config: client_config.clone(),
        };

        self.clients.write().await.insert(federation_id, client);

        let mut dbtx = self.db.begin_transaction().await;
        dbtx.insert_entry(
            &FederationConfigKey { id: federation_id },
            &federation_config,
        )
        .await;
        dbtx.commit_tx().await;

        Ok(FederationSelector {
            federation_name,
            federation_id,
            network,
        })
    }

    async fn build_client(
        &self,
        federation_id: &FederationId,
        invite_code: &InviteCode,
        connector: Connector,
        client_type: ClientType,
    ) -> anyhow::Result<ClientHandleArc> {
        info_to_flutter(format!("Building new client. type: {client_type}")).await;
        let client_db = match client_type {
            ClientType::Temporary => MemDatabase::new().into(),
            _ => self.get_client_database(&federation_id),
        };

        let secret = Self::derive_federation_secret(&self.mnemonic, &federation_id);
        let mut client_builder = Client::builder(client_db).await?;
        client_builder.with_module_inits(self.modules.clone());
        client_builder.with_primary_module_kind(fedimint_mint_client::KIND);

        let client = match client_type {
            ClientType::Recovery { client_config } => {
                let backup = client_builder
                    .download_backup_from_federation(
                        &secret,
                        &client_config,
                        invite_code.api_secret(),
                    )
                    .await?;
                let client = client_builder
                    .recover(secret, client_config, invite_code.api_secret(), backup)
                    .await
                    .map(Arc::new)?;
                self.spawn_recovery_progress(client.clone());
                client
            }
            client_type => {
                let client = if Client::is_initialized(client_builder.db_no_decoders()).await {
                    info_to_flutter("Client is already initialized, opening using secret...").await;
                    client_builder.open(secret).await
                } else {
                    info_to_flutter("Client is not initialized, downloading invite code...").await;
                    let client_config = connector.download_from_invite_code(&invite_code).await?;
                    client_builder
                        .join(secret, client_config.clone(), invite_code.api_secret())
                        .await
                }
                .map(Arc::new)?;

                if client_type == ClientType::New {
                    self.lnv1_update_gateway_cache(&client).await?;
                }

                client
            }
        };

        Ok(client)
    }

    fn spawn_recovery_progress(&self, client: ClientHandleArc) {
        let mut self_copy = self.clone();
        let recovering_client = client.clone();
        self.task_group
            .spawn_cancellable("wait for recovery", async move {
                if let Err(e) = self_copy.wait_for_recovery(recovering_client).await {
                    error_to_flutter(format!("Error waiting for recovery: {e:?}")).await;
                }
            });

        let progress_copy = self.clone();
        self.task_group
            .spawn_cancellable("recovery progress", async move {
                progress_copy
                    .init_recovery_progress_cache(client.federation_id())
                    .await;

                let mut stream = client.subscribe_to_recovery_progress();
                while let Some((module_id, progress)) = stream.next().await {
                    progress_copy
                        .update_recovery_progress_cache(
                            &client.federation_id(),
                            module_id,
                            progress,
                        )
                        .await;
                }

                progress_copy
                    .remove_recovery_progress_cache(&client.federation_id())
                    .await;
            });
    }

    async fn init_recovery_progress_cache(&self, federation_id: FederationId) {
        let mut progress = self.recovery_progress.write().await;
        progress.insert(federation_id, BTreeMap::new());
    }

    async fn remove_recovery_progress_cache(&self, federation_id: &FederationId) {
        let mut progress = self.recovery_progress.write().await;
        progress.remove(federation_id);
    }

    async fn update_recovery_progress_cache(
        &self,
        federation_id: &FederationId,
        module_id: u16,
        module_progress: RecoveryProgress,
    ) {
        let mut progress = self.recovery_progress.write().await;
        if let Some(module_progress_cache) = progress.get_mut(federation_id) {
            module_progress_cache.insert(module_id, module_progress);
        }
        get_event_bus()
            .publish(MultimintEvent::RecoveryProgress(
                federation_id.to_string(),
                module_id,
                module_progress.complete,
                module_progress.total,
            ))
            .await;
    }

    pub async fn get_recovery_progress(
        &self,
        federation_id: &FederationId,
        module_id: u16,
    ) -> RecoveryProgress {
        let progress = self.recovery_progress.read().await;
        let module_progress = progress.get(federation_id);
        if let Some(module_progress) = module_progress {
            if let Some(progress) = module_progress.get(&module_id) {
                return *progress;
            }
        }

        RecoveryProgress {
            complete: 0,
            total: 0,
        }
    }

    async fn wait_for_recovery(
        &mut self,
        recovering_client: ClientHandleArc,
    ) -> anyhow::Result<()> {
        let federation_id = recovering_client.federation_id();
        info_to_flutter("Waiting for all recoveries...").await;
        recovering_client.wait_for_all_recoveries().await?;

        // Try all federation invite codes in case some peers are down
        let config = recovering_client.config().await;
        let peers = config.global.api_endpoints.keys().collect::<Vec<_>>();
        let mut joined = false;
        for peer in peers {
            if let Some(invite_code) = recovering_client.invite_code(peer.clone()).await {
                self.join_federation(invite_code.to_string(), false).await?;
                joined = true;
                break;
            }
        }

        if !joined {
            bail!("Could not re-join federation after recovering");
        }

        let new_client = self
            .clients
            .read()
            .await
            .get(&federation_id)
            .expect("Client should be available")
            .clone();
        info_to_flutter("Waiting for all active state machines...").await;
        new_client.wait_for_all_active_state_machines().await?;

        get_event_bus()
            .publish(MultimintEvent::RecoveryDone(federation_id.to_string()))
            .await;

        Ok(())
    }

    fn get_client_database(&self, federation_id: &FederationId) -> Database {
        let mut prefix = vec![crate::db::DbKeyPrefix::ClientDatabase as u8];
        prefix.append(&mut federation_id.consensus_encode_to_vec());
        self.db.with_prefix(prefix)
    }

    /// Derives a per-federation secret according to Fedimint's multi-federation
    /// secret derivation policy.
    fn derive_federation_secret(
        mnemonic: &Mnemonic,
        federation_id: &FederationId,
    ) -> DerivableSecret {
        let global_root_secret = Bip39RootSecretStrategy::<12>::to_root_secret(mnemonic);
        let multi_federation_root_secret = global_root_secret.child_key(ChildId(0));
        let federation_root_secret = multi_federation_root_secret.federation_key(federation_id);
        let federation_wallet_root_secret = federation_root_secret.child_key(ChildId(0));
        federation_wallet_root_secret.child_key(ChildId(0))
    }

    pub async fn federations(&self) -> Vec<(FederationSelector, bool)> {
        let mut dbtx = self.db.begin_transaction_nc().await;
        dbtx.find_by_prefix(&FederationConfigKeyPrefix)
            .await
            .then(|(id, config)| {
                let clients_clone = self.clients.clone();
                async move {
                    let client = clients_clone
                        .read()
                        .await
                        .get(&id.id)
                        .expect("No client exists")
                        .clone();
                    let selector = FederationSelector {
                        federation_name: config.federation_name,
                        federation_id: id.id,
                        network: config.network,
                    };
                    (selector, client.has_pending_recoveries())
                }
            })
            .collect::<Vec<_>>()
            .await
    }

    pub async fn balance(&self, federation_id: &FederationId) -> u64 {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        client.get_balance().await.msats
    }

    pub async fn receive(
        &self,
        federation_id: &FederationId,
        amount_msats_with_fees: u64,
        amount_msats_without_fees: u64,
        gateway: SafeUrl,
        is_lnv2: bool,
    ) -> anyhow::Result<(Bolt11Invoice, OperationId)> {
        let amount_with_fees = Amount::from_msats(amount_msats_with_fees);
        let amount_without_fees = Amount::from_msats(amount_msats_without_fees);
        info_to_flutter(format!("Amount with fees: {amount_with_fees:?}")).await;
        info_to_flutter(format!("Amount without fees: {amount_without_fees:?}")).await;
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();

        if is_lnv2 {
            if let Ok((invoice, operation_id)) = Self::receive_lnv2(
                &client,
                amount_with_fees,
                amount_without_fees,
                gateway.clone(),
            )
            .await
            {
                info_to_flutter("Using LNv2 for the actual invoice").await;
                return Ok((invoice, operation_id));
            }
        }

        info_to_flutter("Using LNv1 for the actual invoice").await;
        let (invoice, operation_id) =
            Self::receive_lnv1(&client, amount_with_fees, amount_without_fees, gateway).await?;

        // Spawn new task that awaits the payment in case the user clicks away
        self.spawn_await_receive(federation_id.clone(), operation_id.clone());

        Ok((invoice, operation_id))
    }

    fn spawn_await_receive(&self, federation_id: FederationId, operation_id: OperationId) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("await receive", async move {
                match self_copy.await_receive(&federation_id, operation_id).await {
                    Ok((final_state, amount_msats)) => {
                        let lightning_event =
                            LightningEventKind::InvoicePaid(InvoicePaidEvent { amount_msats });
                        info_to_flutter(format!("Receive completed: {final_state:?}")).await;
                        let multimint_event =
                            MultimintEvent::Lightning((federation_id, lightning_event));
                        get_event_bus().publish(multimint_event).await;
                    }
                    Err(e) => {
                        info_to_flutter(format!("Could not await receive {operation_id:?} {e:?}"))
                            .await;
                    }
                }
            });
    }

    async fn receive_lnv2(
        client: &ClientHandleArc,
        amount_with_fees: Amount,
        amount_without_fees: Amount,
        gateway: SafeUrl,
    ) -> anyhow::Result<(Bolt11Invoice, OperationId)> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let custom_meta = json!({
            "amount": amount_without_fees,
            "amount_with_fees": amount_with_fees,
            "gateway_url": gateway,
        });
        let (invoice, operation_id) = lnv2
            .receive(
                amount_with_fees,
                DEFAULT_EXPIRY_TIME_SECS,
                Bolt11InvoiceDescription::Direct(String::new()),
                Some(gateway),
                custom_meta,
            )
            .await?;
        Ok((invoice, operation_id))
    }

    async fn receive_lnv1(
        client: &ClientHandleArc,
        amount_with_fees: Amount,
        amount_without_fees: Amount,
        gateway_url: SafeUrl,
    ) -> anyhow::Result<(Bolt11Invoice, OperationId)> {
        let lnv1 = client.get_first_module::<LightningClientModule>()?;
        let custom_meta = json!({
            "amount": amount_without_fees,
            "amount_with_fees": amount_with_fees,
            "gateway_url": gateway_url,
        });
        let gateways = lnv1.list_gateways().await;
        let gateway = gateways
            .iter()
            .find(|g| g.info.api == gateway_url)
            .ok_or(anyhow!("Could not find gateway"))?
            .info
            .clone();
        let desc = Description::new(String::new())?;
        let (operation_id, invoice, _) = lnv1
            .create_bolt11_invoice(
                amount_with_fees,
                lightning_invoice::Bolt11InvoiceDescription::Direct(&desc),
                Some(DEFAULT_EXPIRY_TIME_SECS as u64),
                custom_meta,
                Some(gateway),
            )
            .await?;
        Ok((invoice, operation_id))
    }

    pub async fn select_receive_gateway(
        &self,
        federation_id: &FederationId,
        amount: Amount,
    ) -> anyhow::Result<(String, u64, bool)> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        if let Ok((url, receive_fee)) = Self::lnv2_select_gateway(&client, None).await {
            // TODO: It is currently not possible to get the fed_base and fed_ppm from the config
            info_to_flutter("Using LNv2 for selecting receive gateway").await;
            let amount_with_fees = compute_receive_amount(
                amount,
                1_000,
                100,
                receive_fee.base.msats,
                receive_fee.parts_per_million,
            );
            return Ok((url.to_string(), amount_with_fees, true));
        }

        // LNv1 does not have fees for receiving
        info_to_flutter("Using LNv1 for selecting receive gateway").await;
        let gateway = Self::lnv1_select_gateway(&client)
            .await
            .ok_or(anyhow!("No available gateways"))?;
        Ok((gateway.api.to_string(), amount.msats, false))
    }

    pub async fn select_send_gateway(
        &self,
        federation_id: &FederationId,
        amount: Amount,
        bolt11: Bolt11Invoice,
    ) -> anyhow::Result<(String, u64, bool)> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        if let Ok((url, send_fee)) = Self::lnv2_select_gateway(&client, Some(bolt11.clone())).await
        {
            let amount_with_fees = compute_send_amount(amount, 1_000, 100, send_fee);
            return Ok((url.to_string(), amount_with_fees, true));
        }

        // LNv1 only has Lightning routing fees
        let gateway = Self::lnv1_select_gateway(&client)
            .await
            .ok_or(anyhow!("No available gateways"))?;
        let fees = if Self::invoice_routes_back_to_federation(&bolt11, gateway.clone()) {
            // There are no fees on internal swaps
            PaymentFee {
                base: Amount::ZERO,
                parts_per_million: 0,
            }
        } else {
            gateway.fees.into()
        };
        let amount_with_fees = compute_send_amount(amount, 0, 0, fees);
        Ok((gateway.api.to_string(), amount_with_fees, false))
    }

    fn invoice_routes_back_to_federation(
        invoice: &Bolt11Invoice,
        gateway: LightningGateway,
    ) -> bool {
        invoice
            .route_hints()
            .first()
            .and_then(|rh| rh.0.last())
            .map(|hop| (hop.src_node_id, hop.short_channel_id))
            == Some((gateway.node_pub_key, gateway.federation_index))
    }

    pub async fn send(
        &self,
        federation_id: &FederationId,
        invoice: String,
        gateway: SafeUrl,
        is_lnv2: bool,
        amount_with_fees: u64,
    ) -> anyhow::Result<OperationId> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        let invoice = Bolt11Invoice::from_str(&invoice)?;
        let custom_meta = json!({
            "amount_with_fees": amount_with_fees,
            "gateway_url": gateway,
        });

        if is_lnv2 {
            info_to_flutter("Attempting to pay using LNv2...").await;
            if let Ok(lnv2_operation_id) = Self::pay_lnv2(
                &client,
                invoice.clone(),
                gateway.clone(),
                custom_meta.clone(),
            )
            .await
            {
                info_to_flutter("Successfully initated LNv2 payment").await;
                return Ok(lnv2_operation_id);
            }
        }

        info_to_flutter("Attempting to pay using LNv1...").await;
        let operation_id = Self::pay_lnv1(&client, invoice, gateway, custom_meta).await?;
        info_to_flutter("Successfully initiated LNv1 payment").await;
        self.spawn_await_send(federation_id.clone(), operation_id.clone());
        Ok(operation_id)
    }

    async fn pay_lnv2(
        client: &ClientHandleArc,
        invoice: Bolt11Invoice,
        gateway: SafeUrl,
        custom_meta: serde_json::Value,
    ) -> anyhow::Result<OperationId> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let operation_id = lnv2.send(invoice, Some(gateway), custom_meta).await?;
        Ok(operation_id)
    }

    async fn pay_lnv1(
        client: &ClientHandleArc,
        invoice: Bolt11Invoice,
        gateway_url: SafeUrl,
        custom_meta: serde_json::Value,
    ) -> anyhow::Result<OperationId> {
        let lnv1 = client.get_first_module::<LightningClientModule>()?;
        let gateways = lnv1.list_gateways().await;
        let gateway = gateways
            .iter()
            .find(|g| g.info.api == gateway_url)
            .ok_or(anyhow!("Could not find gateway"))?
            .info
            .clone();
        let outgoing_lightning_payment = lnv1
            .pay_bolt11_invoice(Some(gateway), invoice, custom_meta)
            .await?;
        Ok(outgoing_lightning_payment.payment_type.operation_id())
    }

    fn spawn_await_send(&self, federation_id: FederationId, operation_id: OperationId) {
        let self_copy = self.clone();
        self.task_group.spawn_cancellable("await send", async move {
            let final_state = self_copy.await_send(&federation_id, operation_id).await;
            info_to_flutter(format!("Send completed: {final_state:?}")).await;
        });
    }

    pub async fn await_send(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> LightningSendOutcome {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();

        let send_state = match Self::await_send_lnv2(&client, operation_id).await {
            Ok(lnv2_final_state) => lnv2_final_state,
            Err(_) => Self::await_send_lnv1(&client, operation_id).await,
        };
        send_state
    }

    async fn await_send_lnv2(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<LightningSendOutcome> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let mut updates = lnv2
            .subscribe_send_operation_state_updates(operation_id)
            .await?
            .into_stream();
        let mut final_state = LightningSendOutcome::Failure;
        while let Some(update) = updates.next().await {
            match update {
                SendOperationState::Success(preimage) => {
                    final_state = LightningSendOutcome::Success(preimage.consensus_encode_to_hex());
                }
                SendOperationState::Refunded => {
                    error_to_flutter("LNv2 payment was refunded").await;
                    final_state = LightningSendOutcome::Failure;
                }
                SendOperationState::Failure => {
                    error_to_flutter("LNv2 payment unrecoverable failure").await;
                    final_state = LightningSendOutcome::Failure;
                }
                _ => {}
            }
        }
        Ok(final_state)
    }

    async fn await_send_lnv1(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> LightningSendOutcome {
        let lnv1 = client
            .get_first_module::<LightningClientModule>()
            .expect("LNv1 module not available");
        // First check if its an internal payment
        let mut final_state = None;
        if let Ok(updates) = lnv1.subscribe_internal_pay(operation_id).await {
            let mut stream = updates.into_stream();
            while let Some(update) = stream.next().await {
                match update {
                    InternalPayState::Preimage(preimage) => {
                        final_state = Some(LightningSendOutcome::Success(
                            preimage.0.consensus_encode_to_hex(),
                        ));
                    }
                    InternalPayState::RefundSuccess {
                        out_points: _,
                        error,
                    } => {
                        final_state = Some(LightningSendOutcome::Failure);
                        error_to_flutter(format!("LNv1 internal payment was refunded: {error:?}"))
                            .await;
                    }
                    InternalPayState::FundingFailed { error } => {
                        final_state = Some(LightningSendOutcome::Failure);
                        error_to_flutter(format!(
                            "LNv1 internal payment funding failed: {error:?}"
                        ))
                        .await;
                    }
                    InternalPayState::RefundError {
                        error_message,
                        error,
                    } => {
                        final_state = Some(LightningSendOutcome::Failure);
                        error_to_flutter(format!(
                            "LNv1 internal payment refund error: {error:?} {error_message}"
                        ))
                        .await;
                    }
                    InternalPayState::UnexpectedError(error) => {
                        final_state = Some(LightningSendOutcome::Failure);
                        error_to_flutter(format!(
                            "LNv1 internal payment unexpected error: {error:?}"
                        ))
                        .await;
                    }
                    _ => {}
                }
            }
        }

        if let Some(internal_final_state) = final_state {
            return internal_final_state;
        }

        // If internal fails, check if its an external payment
        if let Ok(updates) = lnv1.subscribe_ln_pay(operation_id).await {
            let mut stream = updates.into_stream();
            while let Some(update) = stream.next().await {
                match update {
                    LnPayState::Success { preimage } => {
                        final_state = Some(LightningSendOutcome::Success(preimage));
                    }
                    LnPayState::Refunded { gateway_error } => {
                        final_state = Some(LightningSendOutcome::Failure);
                        error_to_flutter(format!(
                            "LNv1 external payment was refunded: {gateway_error:?}"
                        ))
                        .await;
                    }
                    LnPayState::UnexpectedError { error_message } => {
                        final_state = Some(LightningSendOutcome::Failure);
                        error_to_flutter(format!(
                            "LNv1 external payment unexpected error: {error_message}"
                        ))
                        .await;
                    }
                    _ => {}
                }
            }
        }

        if let Some(external_final_state) = final_state {
            return external_final_state;
        }

        LightningSendOutcome::Failure
    }

    pub async fn await_receive(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<(FinalReceiveOperationState, u64)> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        let (receive_state, amount) = match Self::await_receive_lnv2(&client, operation_id).await {
            Ok(lnv2_final_state) => lnv2_final_state,
            Err(_) => Self::await_receive_lnv1(&client, operation_id).await?,
        };

        Ok((receive_state, amount))
    }

    async fn await_receive_lnv2(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<(FinalReceiveOperationState, u64)> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let mut updates = lnv2
            .subscribe_receive_operation_state_updates(operation_id)
            .await?
            .into_stream();
        let mut final_state = FinalReceiveOperationState::Failure;
        while let Some(update) = updates.next().await {
            match update {
                ReceiveOperationState::Claimed => {
                    final_state = FinalReceiveOperationState::Claimed;
                }
                ReceiveOperationState::Expired => {
                    final_state = FinalReceiveOperationState::Expired;
                }
                ReceiveOperationState::Failure => {
                    final_state = FinalReceiveOperationState::Failure;
                }
                _ => {}
            }
        }

        let operation = client.operation_log().get_operation(operation_id).await;
        let amount = Self::get_lnv2_amount_from_meta(operation);
        Ok((final_state, amount))
    }

    fn get_lnv2_amount_from_meta(op_log_val: Option<OperationLogEntry>) -> u64 {
        let Some(op_log_val) = op_log_val else {
            return 0;
        };
        let meta = op_log_val.meta::<LightningOperationMeta>();
        match meta {
            LightningOperationMeta::Receive(receive) => {
                serde_json::from_value::<Amount>(receive.custom_meta)
                    .expect("Could not deserialize amount")
                    .msats
            }
            LightningOperationMeta::Send(send) => send.contract.amount.msats,
        }
    }

    async fn await_receive_lnv1(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<(FinalReceiveOperationState, u64)> {
        let lnv1 = client.get_first_module::<LightningClientModule>()?;
        let mut updates = lnv1.subscribe_ln_receive(operation_id).await?.into_stream();
        let mut final_state = FinalReceiveOperationState::Failure;
        while let Some(update) = updates.next().await {
            match update {
                LnReceiveState::Claimed => {
                    final_state = FinalReceiveOperationState::Claimed;
                }
                _ => {}
            }
        }

        let operation = client.operation_log().get_operation(operation_id).await;
        let amount = Self::get_lnv1_amount_from_meta(operation);
        Ok((final_state, amount))
    }

    async fn spawn_await_recurringd_receive(
        &self,
        client: ClientHandleArc,
        operation_id: OperationId,
        federation_id: FederationId,
    ) {
        self.task_group
            .spawn_cancellable("recurringd invoice", async move {
                info_to_flutter(format!(
                    "Checking invoice with operation id: {operation_id:?}"
                ))
                .await;
                if let Ok(lnv1) = client.get_first_module::<LightningClientModule>() {
                    if let Ok(updates) = lnv1.subscribe_ln_recurring_receive(operation_id).await {
                        let mut stream = updates.into_stream();
                        let mut final_state = FinalReceiveOperationState::Failure;
                        let operation = client
                            .operation_log()
                            .get_operation(operation_id)
                            .await
                            .expect("operation must exist");
                        while let Some(update) = stream.next().await {
                            match update {
                                LnReceiveState::Claimed => {
                                    final_state = FinalReceiveOperationState::Claimed;
                                    if let LightningOperationMetaVariant::RecurringPaymentReceive(
                                        meta,
                                    ) = operation
                                        .meta::<fedimint_ln_client::LightningOperationMeta>()
                                        .variant
                                    {
                                        let amount_msats = meta
                                            .invoice
                                            .amount_milli_satoshis()
                                            .expect("Amount not present");
                                        let lightning_event =
                                            LightningEventKind::InvoicePaid(InvoicePaidEvent {
                                                amount_msats,
                                            });
                                        info_to_flutter(format!(
                                            "Recurringd receive completed: {final_state:?}"
                                        ))
                                        .await;
                                        let multimint_event = MultimintEvent::Lightning((
                                            federation_id,
                                            lightning_event,
                                        ));
                                        get_event_bus().publish(multimint_event).await;
                                    }
                                }
                                _ => {}
                            }
                        }
                        info_to_flutter(format!(
                            "Final state of recurringd receive: {final_state:?}"
                        ))
                        .await;
                    }
                }
            });

        let mut recurringd_invoices = self.recurringd_invoices.write().await;
        recurringd_invoices.insert(operation_id);
    }

    fn get_lnv1_amount_from_meta(op_log_val: Option<OperationLogEntry>) -> u64 {
        let Some(op_log_val) = op_log_val else {
            return 0;
        };

        let meta = op_log_val.meta::<fedimint_ln_client::LightningOperationMeta>();
        match meta.variant {
            LightningOperationMetaVariant::Pay(send) => send
                .invoice
                .amount_milli_satoshis()
                .expect("Cannot pay amountless invoice"),
            LightningOperationMetaVariant::Receive { invoice, .. } => invoice
                .amount_milli_satoshis()
                .expect("Cannot receive amountless invoice"),
            LightningOperationMetaVariant::RecurringPaymentReceive(recurring) => recurring
                .invoice
                .amount_milli_satoshis()
                .expect("Cannot receive amountless invoice"),
            // Claim is covered by send
            _ => 0,
        }
    }

    async fn lnv1_update_gateway_cache(&self, client: &ClientHandleArc) -> anyhow::Result<()> {
        let lnv1_client = client.clone();
        self.task_group
            .spawn_cancellable("update gateway cache", async move {
                let lnv1 = lnv1_client
                    .get_first_module::<LightningClientModule>()
                    .expect("LNv1 should be present");
                match lnv1.update_gateway_cache().await {
                    Ok(_) => info_to_flutter("Updated gateway cache").await,
                    Err(e) => info_to_flutter(format!("Could not update gateway cache {e}")).await,
                }

                lnv1.update_gateway_cache_continuously(|gateway| async { gateway })
                    .await
            });
        Ok(())
    }

    async fn lnv1_select_gateway(
        client: &ClientHandleArc,
    ) -> Option<fedimint_ln_common::LightningGateway> {
        let lnv1 = client.get_first_module::<LightningClientModule>().ok()?;
        let gateways = lnv1.list_gateways().await;

        if gateways.len() == 0 {
            return None;
        }

        if let Some(vetted) = gateways.iter().find(|gateway| gateway.vetted) {
            return Some(vetted.info.clone());
        }

        gateways
            .choose(&mut thread_rng())
            .map(|gateway| gateway.info.clone())
    }

    async fn lnv2_select_gateway(
        client: &ClientHandleArc,
        invoice: Option<Bolt11Invoice>,
    ) -> anyhow::Result<(SafeUrl, PaymentFee)> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let (gateway, routing_info) = lnv2.select_gateway(invoice.clone()).await?;
        let fee = if let Some(bolt11) = invoice {
            if bolt11.get_payee_pub_key() == routing_info.lightning_public_key {
                routing_info.send_fee_minimum
            } else {
                routing_info.send_fee_default
            }
        } else {
            routing_info.receive_fee
        };

        Ok((gateway, fee))
    }

    pub async fn transactions(
        &self,
        federation_id: &FederationId,
        timestamp: Option<u64>,
        operation_id: Option<Vec<u8>>,
        modules: Vec<String>,
    ) -> Vec<Transaction> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();

        let mut collected = Vec::new();
        let mut next_key = if let Some(timestamp) = timestamp {
            Some(ChronologicalOperationLogKey {
                creation_time: UNIX_EPOCH + Duration::from_millis(timestamp),
                operation_id: OperationId(
                    operation_id
                        .expect("Invalid operation")
                        .try_into()
                        .expect("Invalid operation"),
                ),
            })
        } else {
            None
        };

        while collected.len() < 10 {
            let page = client
                .operation_log()
                .paginate_operations_rev(50, next_key.clone())
                .await;

            if page.is_empty() {
                break;
            }

            for (key, op_log_val) in &page {
                if collected.len() >= 10 {
                    break;
                }

                if !modules.contains(&op_log_val.operation_module_kind().to_string()) {
                    continue;
                }

                let timestamp = key
                    .creation_time
                    .duration_since(UNIX_EPOCH)
                    .expect("Cannot be before unix epoch")
                    .as_millis() as u64;

                let tx = match op_log_val.operation_module_kind() {
                    "lnv2" => {
                        let meta = op_log_val.meta::<LightningOperationMeta>();
                        match meta {
                            LightningOperationMeta::Receive(receive) => {
                                let outcome = op_log_val.outcome::<ReceiveOperationState>();
                                let fedimint_lnv2_common::LightningInvoice::Bolt11(bolt11) =
                                    receive.invoice;
                                if let Some(ReceiveOperationState::Claimed) = outcome {
                                    let amount = from_value::<Amount>(
                                        receive
                                            .custom_meta
                                            .get("amount")
                                            .expect("Field missing lightning receive custom meta")
                                            .clone(),
                                    )
                                    .expect("Could not parse to Amount")
                                    .msats;
                                    let amount_with_fees = from_value::<Amount>(
                                        receive
                                            .custom_meta
                                            .get("amount_with_fees")
                                            .expect("Field missing lightning receive custom meta")
                                            .clone(),
                                    )
                                    .expect("Could not parse to Amount")
                                    .msats;
                                    Some(Transaction {
                                        kind: TransactionKind::LightningReceive {
                                            fees: amount_with_fees - amount,
                                            gateway: receive.gateway.to_string(),
                                            payee_pubkey: bolt11.get_payee_pub_key().to_string(),
                                            payment_hash: bolt11.payment_hash().to_string(),
                                        },
                                        amount,
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                } else {
                                    None
                                }
                            }
                            LightningOperationMeta::Send(send) => {
                                let outcome = op_log_val.outcome::<SendOperationState>();
                                let fedimint_lnv2_common::LightningInvoice::Bolt11(bolt11) =
                                    send.invoice;
                                match outcome {
                                    Some(SendOperationState::Success(preimage)) => {
                                        let amount_with_fees = from_value::<u64>(
                                            send.custom_meta
                                                .get("amount_with_fees")
                                                .expect(
                                                    "Field missing lightning receive custom meta",
                                                )
                                                .clone(),
                                        )
                                        .expect("Could not parse to u64");
                                        Some(Transaction {
                                            kind: TransactionKind::LightningSend {
                                                fees: amount_with_fees - send.contract.amount.msats,
                                                gateway: send.gateway.to_string(),
                                                payment_hash: bolt11.payment_hash().to_string(),
                                                preimage: preimage.consensus_encode_to_hex(),
                                            },
                                            amount: send.contract.amount.msats,
                                            timestamp,
                                            operation_id: key.operation_id.0.to_vec(),
                                        })
                                    }
                                    _ => None,
                                }
                            }
                        }
                    }
                    "ln" => {
                        let meta = op_log_val.meta::<fedimint_ln_client::LightningOperationMeta>();
                        match meta.variant {
                            LightningOperationMetaVariant::Pay(send) => Self::get_lnv1_send_tx(
                                send,
                                op_log_val,
                                timestamp,
                                key.operation_id,
                                meta.extra_meta,
                            ),
                            LightningOperationMetaVariant::Receive { invoice, .. } => {
                                Self::get_lnv1_receive_tx(
                                    &invoice,
                                    op_log_val,
                                    timestamp,
                                    key.operation_id,
                                    meta.extra_meta,
                                )
                            }
                            LightningOperationMetaVariant::RecurringPaymentReceive(recurring) => {
                                let amount_msat = recurring
                                    .invoice
                                    .amount_milli_satoshis()
                                    .expect("Amountless invoice");
                                Some(Transaction {
                                    kind: TransactionKind::LightningRecurring,
                                    amount: amount_msat,
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                })
                            }
                            _ => None,
                        }
                    }
                    "mint" => {
                        let meta = op_log_val.meta::<MintOperationMeta>();
                        match meta.variant {
                            MintOperationMetaVariant::SpendOOB { oob_notes, .. } => {
                                let internal_spends = self.internal_ecash_spends.read().await;
                                if internal_spends.contains(&key.operation_id) {
                                    continue;
                                }
                                Some(Transaction {
                                    kind: TransactionKind::EcashSend {
                                        oob_notes: oob_notes.to_string(),
                                        fees: 0, // currently no fees for the mint module
                                    },
                                    amount: oob_notes.total_amount().msats,
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                })
                            }
                            MintOperationMetaVariant::Reissuance { .. } => {
                                let extra_meta = meta.extra_meta.clone();
                                if let Ok(operation_id) =
                                    serde_json::from_value::<OperationId>(extra_meta)
                                {
                                    let mut internal_spends =
                                        self.internal_ecash_spends.write().await;
                                    internal_spends.insert(operation_id);
                                    continue;
                                }

                                let outcome = op_log_val.outcome::<ReissueExternalNotesState>();
                                if let Some(ReissueExternalNotesState::Done) = outcome {
                                    let amount = from_value::<Amount>(
                                        meta.extra_meta
                                            .get("total_amount")
                                            .expect("Field missing ecash custom meta")
                                            .clone(),
                                    )
                                    .expect("Could not parse to Amount");
                                    let ecash = from_value::<String>(
                                        meta.extra_meta
                                            .get("ecash")
                                            .expect("Field missing ecash custom meta")
                                            .clone(),
                                    )
                                    .expect("Could not parse to Amount");
                                    Some(Transaction {
                                        kind: TransactionKind::EcashReceive {
                                            oob_notes: ecash,
                                            fees: 0,
                                        },
                                        amount: amount.msats,
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                } else {
                                    None
                                }
                            }
                        }
                    }
                    "wallet" => {
                        let meta = op_log_val.meta::<WalletOperationMeta>();
                        match meta.variant {
                            WalletOperationMetaVariant::Deposit { .. } => {
                                let outcome = op_log_val.outcome::<DepositStateV2>();
                                if let Some(DepositStateV2::Claimed { btc_deposited, .. }) = outcome
                                {
                                    let amount = Amount::from_sats(btc_deposited.to_sat()).msats;
                                    Some(Transaction {
                                        kind: TransactionKind::OnchainReceive,
                                        amount,
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                } else {
                                    None
                                }
                            }
                            WalletOperationMetaVariant::Withdraw { amount, .. } => {
                                let outcome = op_log_val.outcome::<WithdrawState>();
                                if let Some(WithdrawState::Succeeded(_txid)) = outcome {
                                    Some(Transaction {
                                        kind: TransactionKind::OnchainSend,
                                        amount: Amount::from_sats(amount.to_sat()).msats,
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                } else {
                                    None
                                }
                            }
                            WalletOperationMetaVariant::RbfWithdraw { .. } => {
                                // RbfWithdrawal isn't supported
                                None
                            }
                        }
                    }
                    _ => None,
                };

                if let Some(tx) = tx {
                    collected.push(tx);
                }
            }

            // Update the pagination key to the last item in this page
            next_key = page.last().map(|(key, _)| key.clone());
        }

        collected
    }

    /// LNv1 has two different operation send types: external (over the Lightning network) and internal (ecash swap)
    /// In order to check if the "send" was successful or not, we need to check both outcomes.
    fn get_lnv1_send_tx(
        meta: LightningOperationMetaPay,
        ln_outcome: &OperationLogEntry,
        timestamp: u64,
        operation_id: OperationId,
        custom_meta: serde_json::Value,
    ) -> Option<Transaction> {
        let amount = meta
            .invoice
            .amount_milli_satoshis()
            .expect("Cannot pay amountless invoice");
        let amount_with_fees = from_value::<u64>(
            custom_meta
                .get("amount_with_fees")
                .expect("Field missing lightning receive custom meta")
                .clone(),
        )
        .expect("Could not parse to u64");
        let gateway = from_value::<SafeUrl>(
            custom_meta
                .get("gateway_url")
                .expect("Field missing lightning receive custom meta")
                .clone(),
        )
        .expect("Could not parse SafeUrl")
        .to_string();
        let operation_id = operation_id.0.to_vec();

        // First check if the send was over the Lightning network
        let external_outcome = ln_outcome.outcome::<LnPayState>();
        match external_outcome {
            Some(state) => match state {
                LnPayState::Success { preimage } => Some(Transaction {
                    kind: TransactionKind::LightningSend {
                        fees: amount_with_fees - amount,
                        gateway,
                        payment_hash: meta.invoice.payment_hash().to_string(),
                        preimage,
                    },
                    amount,
                    timestamp,
                    operation_id,
                }),
                _ => None,
            },
            None => {
                // If unsuccessful, check if the payment was an internal payment
                let internal_outcome = ln_outcome.outcome::<InternalPayState>();
                match internal_outcome {
                    Some(state) => match state {
                        InternalPayState::Preimage(preimage) => Some(Transaction {
                            kind: TransactionKind::LightningSend {
                                fees: amount_with_fees - amount,
                                gateway,
                                payment_hash: meta.invoice.payment_hash().to_string(),
                                preimage: preimage.0.consensus_encode_to_hex(),
                            },
                            amount,
                            timestamp,
                            operation_id,
                        }),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    }

    /// Checks the outcome of an LNv1 receive operation and constructs the appropriate `Transaction`
    /// for the transaction log.
    fn get_lnv1_receive_tx(
        invoice: &Bolt11Invoice,
        ln_outcome: &OperationLogEntry,
        timestamp: u64,
        operation_id: OperationId,
        custom_meta: serde_json::Value,
    ) -> Option<Transaction> {
        let receive_outcome = ln_outcome.outcome::<LnReceiveState>();
        let amount = from_value::<Amount>(
            custom_meta
                .get("amount")
                .expect("Field missing lightning receive custom meta")
                .clone(),
        )
        .expect("Could not parse to Amount")
        .msats;
        let amount_with_fees = from_value::<Amount>(
            custom_meta
                .get("amount_with_fees")
                .expect("Field missing lightning receive custom meta")
                .clone(),
        )
        .expect("Could not parse to Amount")
        .msats;
        let gateway = from_value::<SafeUrl>(
            custom_meta
                .get("gateway_url")
                .expect("Field missing lightning receive custom meta")
                .clone(),
        )
        .expect("Could not parse SafeUrl")
        .to_string();
        match receive_outcome {
            Some(state) if state == LnReceiveState::Claimed => Some(Transaction {
                kind: TransactionKind::LightningReceive {
                    fees: amount_with_fees - amount,
                    gateway,
                    payee_pubkey: invoice.get_payee_pub_key().to_string(),
                    payment_hash: invoice.payment_hash().to_string(),
                },
                amount,
                timestamp,
                operation_id: operation_id.0.to_vec(),
            }),
            _ => None,
        }
    }

    async fn spend_until_exact_amount(
        &self,
        client: &ClientHandleArc,
        amount_msats: u64,
    ) -> anyhow::Result<(OOBNotes, OperationId)> {
        let amount = Amount::from_msats(amount_msats);
        let mint = client.get_first_module::<MintClientModule>()?;
        // Default timeout after one day
        let timeout = Duration::from_secs(60 * 60 * 24);
        loop {
            let (operation_id, notes) = mint
                .spend_notes_with_selector(&SelectNotesWithAtleastAmount, amount, timeout, true, ())
                .await?;

            if notes.total_amount() == amount {
                return Ok((notes, operation_id));
            }

            // reissue the notes back to ourselves
            let reissue_op_id = mint.reissue_external_notes(notes, operation_id).await?;
            self.await_ecash_reissue(&client.federation_id(), reissue_op_id)
                .await?;
        }
    }

    pub async fn send_ecash(
        &self,
        federation_id: &FederationId,
        amount_msats: u64,
    ) -> anyhow::Result<(OperationId, String, u64)> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .ok_or(anyhow!("Federation does not exist"))?
            .clone();
        let (notes, operation_id) = self.spend_until_exact_amount(&client, amount_msats).await?;
        self.spawn_await_ecash_send(*federation_id, operation_id);
        let serialized_notes = notes.to_string();
        info_to_flutter(format!("Ecash note size: {}", serialized_notes.len())).await;
        Ok((operation_id, serialized_notes, notes.total_amount().msats))
    }

    fn spawn_await_ecash_send(&self, federation_id: FederationId, operation_id: OperationId) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("await ecash send", async move {
                match self_copy
                    .await_ecash_send(&federation_id, operation_id)
                    .await
                {
                    Ok(final_state) => {
                        info_to_flutter(format!("Ecash send completed: {final_state:?}")).await;
                    }
                    Err(e) => {
                        info_to_flutter(format!("Could not await receive {operation_id:?} {e:?}"))
                            .await;
                    }
                }
            });
    }

    pub async fn await_ecash_send(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<SpendOOBState> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .ok_or(anyhow!("No federation exists"))?
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let mut updates = mint
            .subscribe_spend_notes(operation_id)
            .await?
            .into_stream();
        let mut final_state = SpendOOBState::UserCanceledFailure;
        while let Some(update) = updates.next().await {
            final_state = update;
        }
        Ok(final_state)
    }

    pub async fn parse_ecash(
        &self,
        federation_id: &FederationId,
        ecash: String,
    ) -> anyhow::Result<u64> {
        let notes = OOBNotes::from_str(&ecash)?;
        let given_federation_id_prefix = notes.federation_id_prefix();
        if federation_id.to_prefix() != given_federation_id_prefix {
            return Err(anyhow!("Trying to claim ecash into incorrect federation"));
        }
        let total_amount = notes.total_amount();
        Ok(total_amount.msats)
    }

    pub async fn check_ecash_spent(
        &self,
        federation_id: &FederationId,
        ecash: String,
    ) -> anyhow::Result<bool> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .ok_or(anyhow!("No federation exists"))?
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let oob_notes = OOBNotes::from_str(&ecash)?;
        // We assume that if any note has been spent, all of the notes have been spent
        for (amount, notes) in oob_notes.notes().iter() {
            info_to_flutter(format!("Checking if notes in tier {:?} are spent", amount)).await;
            for note in notes {
                let nonce = note.nonce();
                if mint.api.check_note_spent(nonce).await? {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub async fn reissue_ecash(
        &self,
        federation_id: &FederationId,
        ecash: String,
    ) -> anyhow::Result<OperationId> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .ok_or(anyhow!("No federation exists"))?
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let notes = OOBNotes::from_str(&ecash)?;
        let total_amount = notes.total_amount();
        let extra_meta = json!({
            "total_amount": total_amount,
            "ecash": ecash,
        });
        let operation_id = mint.reissue_external_notes(notes, extra_meta).await?;
        self.spawn_await_ecash_reissue(federation_id.clone(), operation_id);
        Ok(operation_id)
    }

    fn spawn_await_ecash_reissue(&self, federation_id: FederationId, operation_id: OperationId) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("await ecash reissue", async move {
                match self_copy
                    .await_ecash_reissue(&federation_id, operation_id)
                    .await
                {
                    Ok((final_state, amount)) => {
                        info_to_flutter(format!("Ecash reissue completed: {final_state:?}")).await;
                        if let Some(amount) = amount {
                            let ecash_event = MultimintEvent::Ecash((federation_id, amount));
                            get_event_bus().publish(ecash_event).await;
                        }
                    }
                    Err(e) => {
                        info_to_flutter(format!("Could not await receive {operation_id:?} {e:?}"))
                            .await;
                    }
                }
            });
    }

    pub async fn await_ecash_reissue(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<(ReissueExternalNotesState, Option<u64>)> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .ok_or(anyhow!("No federation exists"))?
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let mut updates = mint
            .subscribe_reissue_external_notes(operation_id)
            .await?
            .into_stream();
        let mut final_state = ReissueExternalNotesState::Failed("Unexpected state".to_string());
        while let Some(update) = updates.next().await {
            match update {
                ReissueExternalNotesState::Done => {
                    final_state = ReissueExternalNotesState::Done;
                }
                ReissueExternalNotesState::Failed(e) => {
                    final_state = ReissueExternalNotesState::Failed(e);
                }
                _ => {}
            }
        }

        let operation = client.operation_log().get_operation(operation_id).await;
        let amount = Self::get_ecash_amount_from_meta(operation);

        Ok((final_state, amount))
    }

    fn get_ecash_amount_from_meta(op_log_val: Option<OperationLogEntry>) -> Option<u64> {
        let Some(op_log_val) = op_log_val else {
            return None;
        };
        let meta = op_log_val.meta::<MintOperationMeta>();
        // Internal reissues will have an operation id in the extra meta, these should not generate events
        if serde_json::from_value::<OperationId>(meta.extra_meta).is_ok() {
            return None;
        }

        Some(meta.amount.msats)
    }

    pub async fn calculate_withdraw_fees(
        &self,
        federation_id: &FederationId,
        address: String,
        amount_sats: u64,
    ) -> anyhow::Result<WithdrawFeesResponse> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        let address = bitcoin::address::Address::from_str(&address)?;
        let address = address.require_network(wallet_module.get_network())?;
        let amount = bitcoin::Amount::from_sat(amount_sats);

        let fees = wallet_module.get_withdraw_fees(&address, amount).await?;
        let fee_amount = fees.amount().to_sat();
        let fee_rate_sats_per_vb = fees.fee_rate.sats_per_kvb as f64 / 1000.0;
        // ceil(weight / 4) using only u32
        let tx_size_vbytes = ((fees.total_weight + 3) / 4) as u32;

        Ok(WithdrawFeesResponse {
            fee_amount,
            fee_rate_sats_per_vb,
            tx_size_vbytes,
            peg_out_fees: fees,
        })
    }

    pub async fn withdraw_to_address(
        &self,
        federation_id: &FederationId,
        address: String,
        amount_sats: u64,
        peg_out_fees: PegOutFees,
    ) -> anyhow::Result<OperationId> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        let address = bitcoin::address::Address::from_str(&address)?;
        let address = address.require_network(wallet_module.get_network())?;
        let amount = bitcoin::Amount::from_sat(amount_sats);

        let operation_id = wallet_module
            .withdraw(&address, amount, peg_out_fees, ())
            .await?;
        Ok(operation_id)
    }

    pub async fn await_withdraw(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<String> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        let mut updates = wallet_module
            .subscribe_withdraw_updates(operation_id)
            .await?
            .into_stream();

        let txid = loop {
            let update = updates
                .next()
                .await
                .ok_or_else(|| anyhow!("Update stream ended without outcome"))?;

            match update {
                WithdrawState::Succeeded(txid) => {
                    // drive the update stream to completion so we get an outcome
                    while updates.next().await.is_some() {}
                    break txid.consensus_encode_to_hex();
                }
                WithdrawState::Failed(e) => {
                    bail!("Withdraw failed: {e}");
                }
                WithdrawState::Created => {
                    continue;
                }
            }
        };

        Ok(txid)
    }

    pub async fn get_max_withdrawable_amount(
        &self,
        federation_id: &FederationId,
        address: String,
    ) -> anyhow::Result<u64> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        let address = bitcoin::address::Address::from_str(&address)?;
        let address = address.require_network(wallet_module.get_network())?;
        let balance = bitcoin::Amount::from_sat(client.get_balance().await.msats / 1000);
        let fees = wallet_module.get_withdraw_fees(&address, balance).await?;
        let max_withdrawable = balance
            .checked_sub(fees.amount())
            .context("Not enough funds to pay fees")?;

        Ok(max_withdrawable.to_sat())
    }

    pub async fn monitor_deposit_address(
        &self,
        federation_id: FederationId,
        address: String,
    ) -> anyhow::Result<()> {
        let client = self
            .clients
            .read()
            .await
            .get(&federation_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No federation exists"))?;

        let wallet_module = client.get_first_module::<WalletClientModule>()?;
        let address = bitcoin::Address::from_str(&address)?;
        let tweak_idx = wallet_module
            .find_tweak_idx_by_address(address.clone())
            .await?;
        let mut addresses = self.allocated_bitcoin_addresses.write().await;
        let fed_addresses = addresses.entry(federation_id).or_insert(BTreeMap::new());
        fed_addresses.insert(tweak_idx, (address.assume_checked().to_string(), None));

        self.pegin_address_monitor_tx
            .send((federation_id, tweak_idx))
            .map_err(|e| anyhow::anyhow!("failed to monitor tweak index: {}", e))?;

        Ok(())
    }

    pub async fn allocate_deposit_address(
        &self,
        federation_id: FederationId,
    ) -> anyhow::Result<String> {
        let client = self
            .clients
            .read()
            .await
            .get(&federation_id)
            .expect("No federation exists")
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        let (_, address, _) = wallet_module.safe_allocate_deposit_address(()).await?;
        self.monitor_deposit_address(federation_id, address.to_string())
            .await?;

        Ok(address.to_string())
    }

    pub async fn wallet_summary(
        &self,
        invite: Option<String>,
        federation_id: Option<FederationId>,
    ) -> anyhow::Result<Vec<Utxo>> {
        let client = match invite {
            Some(invite) => {
                let invite_code = InviteCode::from_str(&invite)?;
                self.get_or_build_temp_client(invite_code).await?.0
            }
            None => {
                let federation_id =
                    federation_id.expect("Invite code and federation ID cannot both be None");
                let clients = self.clients.read().await;
                clients
                    .get(&federation_id)
                    .ok_or(anyhow!("No federation exists"))?
                    .clone()
            }
        };
        let wallet_module = client.get_first_module::<WalletClientModule>()?;
        let wallet_summary = wallet_module.get_wallet_summary().await?;
        let mut utxos: Vec<Utxo> = wallet_summary
            .spendable_utxos
            .into_iter()
            .map(Utxo::from)
            .collect();
        utxos.sort_by_key(|u| std::cmp::Reverse(u.amount));
        Ok(utxos)
    }

    pub async fn get_btc_price(&self) -> Option<u64> {
        let mut dbtx = self.db.begin_transaction_nc().await;
        dbtx.get_value(&BtcPriceKey).await.map(|p| p.price)
    }

    pub async fn get_addresses(
        &self,
        federation_id: &FederationId,
    ) -> Vec<(String, u64, Option<u64>)> {
        let addresses = self.allocated_bitcoin_addresses.read().await;
        if let Some(fed_addresses) = addresses.get(federation_id) {
            let mut res: Vec<_> = fed_addresses
                .iter()
                .map(|(k, v)| (v.0.clone(), k.0, v.1))
                .collect();
            res.sort_by_key(|entry| entry.1);
            res
        } else {
            Vec::new()
        }
    }

    pub async fn recheck_address(
        &self,
        federation_id: &FederationId,
        tweak_idx: u64,
    ) -> anyhow::Result<()> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        wallet_module
            .recheck_pegin_address(TweakIdx(tweak_idx))
            .await?;
        Ok(())
    }

    pub async fn get_note_summary(
        &self,
        federation_id: &FederationId,
    ) -> anyhow::Result<Vec<(u64, usize)>> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();

        let mint = client.get_first_module::<MintClientModule>()?;
        let mut dbtx = client.db().begin_transaction_nc().await;
        let tiered_notes = mint
            .get_note_counts_by_denomination(&mut dbtx.to_ref_with_prefix_module_id(1).0)
            .await;
        let notes = tiered_notes
            .iter()
            .map(|(amount, count)| (amount.msats, count))
            .collect::<Vec<_>>();
        Ok(notes)
    }

    pub async fn list_gateways(
        &self,
        federation_id: &FederationId,
    ) -> anyhow::Result<Vec<FedimintGateway>> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let lnv1 = client.get_first_module::<LightningClientModule>()?;
        let lnv1_gateways = lnv1.list_gateways().await;
        let mut gateways = lnv1_gateways
            .into_iter()
            .map(|g| {
                let info = g.info;
                FedimintGateway {
                    endpoint: info.api.to_string(),
                    base_routing_fee: info.fees.base_msat as u64,
                    ppm_routing_fee: info.fees.proportional_millionths as u64,
                    base_transaction_fee: 0,
                    ppm_transaction_fee: 0,
                    lightning_alias: Some(info.lightning_alias),
                    lightning_node: Some(info.node_pub_key.to_string()),
                }
            })
            .collect::<Vec<_>>();

        // TODO: This only adds 1 LNv2 gateway. Good enough for now, but needs Fedimint changes to display all
        if let Ok(lnv2) = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>() {
            if let Ok((lnv2_api, lnv2_routing_info)) = lnv2.select_gateway(None).await {
                gateways.push(FedimintGateway {
                    endpoint: lnv2_api.to_string(),
                    base_routing_fee: lnv2_routing_info.send_fee_default.base.msats,
                    ppm_routing_fee: lnv2_routing_info.send_fee_default.parts_per_million,
                    base_transaction_fee: lnv2_routing_info.receive_fee.base.msats,
                    ppm_transaction_fee: lnv2_routing_info.receive_fee.parts_per_million,
                    lightning_alias: None,
                    lightning_node: Some(lnv2_routing_info.lightning_public_key.to_string()),
                });
            }
        }

        Ok(gateways)
    }

    /// Retreives currently configured Lightning Address
    pub async fn get_ln_address_config(
        &self,
        federation_id: &FederationId,
    ) -> Option<LightningAddressConfig> {
        let mut dbtx = self.db.begin_transaction_nc().await;
        dbtx.get_value(&LightningAddressKey {
            federation_id: *federation_id,
        })
        .await
    }

    /// Removes an existing LN Address
    async fn remove_existing_ln_address(
        &self,
        federation_id: &FederationId,
        ln_address_api: String,
    ) -> anyhow::Result<()> {
        let mut dbtx = self.db.begin_transaction().await;
        let existing_config = dbtx
            .remove_entry(&LightningAddressKey {
                federation_id: *federation_id,
            })
            .await;
        if let Some(config) = existing_config {
            let safe_ln_address_api = SafeUrl::parse(&ln_address_api)?;
            let remove_request = LNAddressRemoveRequest {
                username: config.username,
                domain: config.domain,
                authentication_token: config.authentication_token,
            };

            let http_client = reqwest::Client::new();
            let remove_endpoint = safe_ln_address_api.join("lnaddress/remove")?;
            let result = http_client
                .post(remove_endpoint.to_unsafe())
                .json(&remove_request)
                .send()
                .await
                .context("Failed to send remove request")?;

            if !result.status().is_success() {
                let status = result.status();
                let body = result.text().await.unwrap_or_default();
                bail!("Failed to remove LN address: {} - {}", status, body);
            }
        }

        Ok(())
    }

    /// Register LNURL/LN Address
    pub async fn register_ln_address(
        &self,
        federation_id: &FederationId,
        recurringd_api: String,
        ln_address_api: String,
        username: String,
        domain: String,
    ) -> anyhow::Result<()> {
        self.remove_existing_ln_address(federation_id, ln_address_api.clone())
            .await?;

        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .context("No federation exists")?
            .clone();
        let lnv1 = client.get_first_module::<LightningClientModule>()?;

        // Verify at least one LNv1 gateway is registered
        let lnv1_gateways = lnv1.list_gateways().await;
        if lnv1_gateways.is_empty() {
            bail!("No LNv1 gateways");
        }

        // First, register an LNURL with recurringd
        let safe_recurringd_api = SafeUrl::parse(&recurringd_api)?;

        let meta = serde_json::to_string(&json!([["text/plain", "Fedimint LNURL Pay"]]))
            .expect("serialization can't fail");

        let lnurl = lnv1
            .register_recurring_payment_code(
                fedimint_ln_client::recurring::RecurringPaymentProtocol::LNURL,
                safe_recurringd_api.clone(),
                meta.as_str(),
            )
            .await?;
        info_to_flutter(format!("Registered LNURL {:?}", lnurl)).await;

        let safe_ln_address_api = SafeUrl::parse(&ln_address_api)?;
        let register_request = LNAddressRegisterRequest {
            username: username.clone(),
            domain: domain.clone(),
            lnurl: lnurl.code.clone(),
        };

        let http_client = reqwest::Client::new();
        let register_endpoint = safe_ln_address_api.join("lnaddress/register")?;
        let result = http_client
            .post(register_endpoint.to_unsafe())
            .json(&register_request)
            .send()
            .await
            .context("Failed to send register request")?;

        if !result.status().is_success() {
            let status = result.status();
            let body = result.text().await.unwrap_or_default();
            bail!("Failed to register LN address: {} - {}", status, body);
        }

        let registration_result = result.json::<serde_json::Value>().await?;
        let authentication_token = registration_result
            .get("authentication_token")
            .ok_or(anyhow!("No authentication token"))?
            .as_str()
            .expect("Authentication token is not a String");
        info_to_flutter(format!("Registration result: {registration_result}")).await;

        let mut dbtx = self.db.begin_transaction().await;
        dbtx.insert_entry(
            &LightningAddressKey {
                federation_id: *federation_id,
            },
            &LightningAddressConfig {
                username,
                domain,
                recurringd_api: safe_recurringd_api,
                ln_address_api: safe_ln_address_api,
                lnurl: lnurl.code.clone(),
                authentication_token: authentication_token.to_string(),
            },
        )
        .await;
        dbtx.commit_tx().await;

        info_to_flutter(format!(
            "Successfully registered LN Address. LNURL: {}",
            lnurl.code
        ))
        .await;

        Ok(())
    }

    // Check LN Address status (registered or not)
    pub async fn check_ln_address_availability(
        &self,
        username: String,
        domain: String,
        ln_address_api: String,
        recurringd_api: String,
        federation_id: &FederationId,
    ) -> anyhow::Result<LNAddressStatus> {
        // First check if the current config is equivalent
        if let Some(current_config) = self.get_ln_address_config(federation_id).await {
            if username == current_config.username && domain == current_config.domain {
                return Ok(LNAddressStatus::CurrentConfig);
            }
        }

        // Check that the selected federation is supported by recurringd
        let supported_federations = self.get_recurringd_federations(recurringd_api).await?;
        if !supported_federations.contains(federation_id) {
            return Ok(LNAddressStatus::UnsupportedFederation);
        }

        // Validate that the given username and domain are a valid Lightning Address
        let username_re = regex::Regex::new(r"^[a-z0-9](?:[a-z0-9._-]*[a-z0-9])?$")?;
        let domain_re = regex::Regex::new(r"^[a-z0-9.-]+\.[a-z]{2,}$")?;

        if !username_re.is_match(&username) {
            return Ok(LNAddressStatus::Invalid);
        }

        if !domain_re.is_match(&domain) {
            return Ok(LNAddressStatus::Invalid);
        }

        let safe_url = SafeUrl::parse(&ln_address_api)?;
        let endpoint = safe_url.join(&format!("lnaddress/{}/{}", domain, username))?;
        let http_client = reqwest::Client::new();
        let result = http_client
            .get(endpoint.to_unsafe())
            .send()
            .await
            .context("Failed to send GET request")?;

        match result.status() {
            StatusCode::OK => {
                let json = result.json::<serde_json::Value>().await?;
                let payment_code = json
                    .get("url")
                    .ok_or(anyhow!("url not in response"))?
                    .as_str()
                    .ok_or(anyhow!("response not a string"))?;
                Ok(LNAddressStatus::Registered {
                    lnurl: payment_code.to_string(),
                })
            }
            StatusCode::NOT_FOUND => Ok(LNAddressStatus::Available),
            _ => {
                error_to_flutter(format!(
                    "Error getting ln address availability: {:?}",
                    result
                ))
                .await;
                Err(anyhow!("Error getting ln address availability"))
            }
        }
    }

    /// Returns a vector of `FederationId`s that recurringd supports
    async fn get_recurringd_federations(
        &self,
        recurringd_api: String,
    ) -> anyhow::Result<Vec<FederationId>> {
        let endpoint = SafeUrl::parse(&recurringd_api)?.join("lnv1/federations")?;

        let http_client = reqwest::Client::new();
        let result = http_client
            .get(endpoint.to_unsafe())
            .send()
            .await
            .context("Failed to send domains request")?;

        let feds = result.json::<Vec<FederationId>>().await?;
        Ok(feds)
    }

    fn spawn_recurring_invoice_listener(&self) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("recurringd listener", async move {
                info_to_flutter("Spawning recurringd invoice listener").await;
                let mut interval = tokio::time::interval(Duration::from_secs(20));
                interval.tick().await;
                loop {
                    let mut dbtx = self_copy.db.begin_transaction_nc().await;
                    let lightning_configs = dbtx
                        .find_by_prefix(&LightningAddressKeyPrefix)
                        .await
                        .collect::<Vec<_>>()
                        .await;
                    for (key, config) in lightning_configs {
                        let federation_id = key.federation_id;
                        if let Some(client) = self_copy.clients.read().await.get(&federation_id) {
                            let lnv1 = client
                                .get_first_module::<LightningClientModule>()
                                .expect("No LNv1 module");
                            let payment_codes = lnv1.list_recurring_payment_codes().await;
                            if let Some((index, _)) = payment_codes
                                .into_iter()
                                .find(|(_, entry)| entry.code == config.lnurl)
                            {
                                if let Some(invoices) =
                                    lnv1.list_recurring_payment_code_invoices(index).await
                                {
                                    for (_, operation_id) in invoices {
                                        let operation = client
                                            .operation_log()
                                            .get_operation(operation_id)
                                            .await
                                            .expect("operation must exist");
                                        if operation.outcome::<serde_json::Value>().is_none()
                                            && !self_copy
                                                .recurringd_invoices
                                                .read()
                                                .await
                                                .contains(&operation_id)
                                        {
                                            self_copy
                                                .spawn_await_recurringd_receive(
                                                    client.clone(),
                                                    operation_id,
                                                    federation_id,
                                                )
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    interval.tick().await;
                }
            });
    }

    pub async fn get_all_invite_codes(&self) -> Vec<String> {
        let mut dbtx = self.db.begin_transaction_nc().await;
        let configs = dbtx
            .find_by_prefix(&FederationConfigKeyPrefix)
            .await
            .collect::<Vec<_>>()
            .await;
        let clients = self.clients.read().await;
        let mut all_invite_codes = Vec::new();
        for (key, config) in configs {
            let client = clients.get(&key.id);
            if let Some(client) = client {
                let peers = config
                    .client_config
                    .global
                    .api_endpoints
                    .keys()
                    .collect::<Vec<_>>();
                for peer in peers {
                    let invite_code = client
                        .invite_code(*peer)
                        .await
                        .expect("Invalid peer")
                        .to_string();
                    all_invite_codes.push(invite_code);
                }
            }
        }

        all_invite_codes
    }

    pub async fn rejoin_from_backup_invites(&mut self, backup_invite_codes: Vec<String>) {
        let mut already_joined_feds = BTreeSet::new();
        for invite in backup_invite_codes {
            if let Ok(invite_code) = InviteCode::from_str(&invite) {
                if !already_joined_feds.contains(&invite_code.federation_id()) {
                    if let Err(e) = self.join_federation(invite.clone(), true).await {
                        error_to_flutter(format!(
                            "Rejoining federation {} with invite code {} failed {}",
                            invite_code.federation_id(),
                            invite,
                            e
                        ))
                        .await;
                    } else {
                        already_joined_feds.insert(invite_code.federation_id());
                        info_to_flutter(format!(
                            "Successfully rejoined {} after recovery",
                            invite_code.federation_id()
                        ))
                        .await;
                    }
                }
            }
        }
    }

    pub async fn get_invite_code(
        &self,
        federation_id: &FederationId,
        peer: u16,
    ) -> anyhow::Result<String> {
        let clients = self.clients.read().await;
        let client = clients
            .get(federation_id)
            .ok_or(anyhow!("Federation does not exist"))?;
        Ok(client
            .invite_code(peer.into())
            .await
            .ok_or(anyhow!("Peer does not exist"))?
            .to_string())
    }

    pub async fn get_display_setting(&self) -> DisplaySetting {
        let mut dbtx = self.db.begin_transaction_nc().await;
        dbtx.get_value(&DisplaySettingKey)
            .await
            .unwrap_or(DisplaySetting::Bip177)
    }

    pub async fn set_display_setting(&self, display_setting: DisplaySetting) {
        let mut dbtx = self.db.begin_transaction().await;
        dbtx.insert_entry(&DisplaySettingKey, &display_setting)
            .await;
        dbtx.commit_tx().await;
    }
}

/// Using the given federation (transaction) and gateway fees, compute the value `X` such that `X - total_fee == requested_amount`.
/// This is non-trivial because the federation and gateway fees both contain a ppm fee, making each fee calculation dependent on each other.
fn compute_receive_amount(
    requested_amount: Amount,
    fed_base: u64,
    fed_ppm: u64,
    gw_base: u64,
    gw_ppm: u64,
) -> u64 {
    let requested_f = requested_amount.msats as f64;
    let fed_base_f = fed_base as f64;
    let fed_ppm_f = fed_ppm as f64;
    let gw_base_f = gw_base as f64;
    let gw_ppm_f = gw_ppm as f64;
    let x_after_gateway = (requested_f + fed_base_f) / (1.0 - fed_ppm_f / 1_000_000.0);
    let x_f = (x_after_gateway + gw_base_f) / (1.0 - gw_ppm_f / 1_000_000.0);
    let x_ceil = receive_amount_after_fees(x_f.ceil() as u64, gw_base, gw_ppm, fed_base, fed_ppm);

    if x_ceil == requested_amount.msats {
        x_f.ceil() as u64
    } else {
        // The above logic is not exactly correct due to rounding, so it could be off by a few msats
        // Until the above math is fixed, just iterate from the overestimate down until we find a value
        // that, after fees, matches the `requested_amount`
        let max = x_f.ceil() as u64;
        let requested = requested_amount.msats;
        for i in (requested..=max).rev() {
            let receive = receive_amount_after_fees(i, gw_base, gw_ppm, fed_base, fed_ppm);
            if receive == requested {
                return i;
            }
        }
        max
    }
}

/// Using the given federation (transaction) and gateway fees, compute amount that will be leftover from `x` after fees
/// have been subtracted.
fn receive_amount_after_fees(
    x: u64,
    gw_base: u64,
    gw_ppm: u64,
    fed_base: u64,
    fed_ppm: u64,
) -> u64 {
    let gw_fee = gw_base + ((gw_ppm as f64 / 1_000_000.0) * x as f64) as u64;
    let after_gateway = x - gw_fee;
    let fed_fee = fed_base + ((fed_ppm as f64 / 1_000_000.0) * after_gateway as f64) as u64;
    let leftover = after_gateway - fed_fee;
    leftover
}

/// Given the `requested_amount`, compute the total that the user will pay including gateway and federation (transaction) fees.
fn compute_send_amount(
    requested_amount: Amount,
    fed_base: u64,
    fed_ppm: u64,
    send_fee: PaymentFee,
) -> u64 {
    let contract_amount = send_fee.add_to(requested_amount.msats);
    let fed_fee =
        fed_base + (((fed_ppm as f64) / 1_000_000.0) * contract_amount.msats as f64) as u64;
    let total = contract_amount.msats + fed_fee;
    total
}

#[cfg(test)]
mod tests {
    use fedimint_lnv2_common::gateway_api::PaymentFee;

    use crate::multimint::{
        compute_receive_amount, compute_send_amount, receive_amount_after_fees,
    };

    #[test]
    fn verify_lnv2_receive_amount() {
        let invoice_amount = compute_receive_amount(
            fedimint_core::Amount::from_sats(1_000),
            1_000,
            100,
            50_000,
            5_000,
        );
        assert_eq!(invoice_amount, 1_056_381);

        let leftover = receive_amount_after_fees(1_056_381, 50_000, 5_000, 1_000, 100);
        assert_eq!(leftover, 1_000_000);

        let invoice_amount = compute_receive_amount(
            fedimint_core::Amount::from_sats(54_561),
            1_000,
            100,
            5_555,
            1_234,
        );
        assert_eq!(invoice_amount, 54_640_437);

        let leftover = receive_amount_after_fees(54_640_437, 5_555, 1_234, 1_000, 100);
        assert_eq!(leftover, 54_561_000);
    }

    #[test]
    fn verify_lnv2_send_amount() {
        let send_amount = compute_send_amount(
            fedimint_core::Amount::from_sats(1_000),
            1_000,
            100,
            PaymentFee {
                base: fedimint_core::Amount::from_sats(50),
                parts_per_million: 5_000,
            },
        );
        assert_eq!(send_amount, 1_056_105);
    }
}
