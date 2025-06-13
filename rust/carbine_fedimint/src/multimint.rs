use std::{
    collections::BTreeMap,
    fmt::Display,
    str::FromStr,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use bitcoin::key::rand::{seq::SliceRandom, thread_rng};
use fedimint_api_client::api::net::Connector;
use fedimint_bip39::{Bip39RootSecretStrategy, Language, Mnemonic};
use fedimint_client::{
    db::ChronologicalOperationLogKey, module_init::ClientModuleInitRegistry,
    secret::RootSecretStrategy, Client, ClientHandleArc, OperationId,
};
use fedimint_core::{
    config::{ClientConfig, FederationId},
    db::{mem_impl::MemDatabase, Database, IDatabaseTransactionOpsCoreTyped},
    encoding::Encodable,
    hex,
    invite_code::InviteCode,
    task::TaskGroup,
    util::SafeUrl,
    Amount,
};
use fedimint_derive_secret::{ChildId, DerivableSecret};
use fedimint_ln_client::{
    InternalPayState, LightningClientInit, LightningClientModule, LightningOperationMetaVariant,
    LnPayState, LnReceiveState,
};
use fedimint_ln_common::LightningGateway;
use fedimint_lnv2_client::{
    FinalReceiveOperationState, FinalSendOperationState, LightningOperationMeta,
    ReceiveOperationState, SendOperationState,
};
use fedimint_lnv2_common::{gateway_api::PaymentFee, Bolt11InvoiceDescription};
use fedimint_meta_client::{common::DEFAULT_META_KEY, MetaClientInit};
use fedimint_mint_client::{
    MintClientInit, MintClientModule, MintOperationMeta, MintOperationMetaVariant, OOBNotes,
    ReissueExternalNotesState, SelectNotesWithAtleastAmount, SpendOOBState,
};
use fedimint_wallet_client::client_db::TweakIdx;
use fedimint_wallet_client::{api::WalletFederationApi, TxOutputSummary};
use fedimint_wallet_client::{
    DepositStateV2, WalletClientInit, WalletClientModule, WalletOperationMeta,
    WalletOperationMetaVariant, WithdrawState,
};
use futures_util::StreamExt;
use lightning_invoice::{Bolt11Invoice, Description};
use serde::Serialize;
use serde_json::to_value;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::RwLock;

use crate::event_bus::EventBus;
use crate::frb_generated::StreamSink;
use crate::{
    anyhow, FederationConfig, FederationConfigKey, FederationConfigKeyPrefix, SeedPhraseAckKey,
};

const DEFAULT_EXPIRY_TIME_SECS: u32 = 86400;

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

#[derive(Clone, Eq, PartialEq, Serialize, Debug)]
pub struct FederationSelector {
    pub federation_name: String,
    pub federation_id: FederationId,
    pub network: Option<String>,
    pub invite_code: String,
}

impl Display for FederationSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.federation_name)
    }
}

#[derive(Clone)]
pub struct Multimint {
    db: Database,
    mnemonic: Mnemonic,
    modules: ClientModuleInitRegistry,
    clients: Arc<RwLock<BTreeMap<FederationId, ClientHandleArc>>>,
    task_group: TaskGroup,
    pegin_address_monitor_tx: UnboundedSender<(FederationId, TweakIdx)>,
    event_bus: EventBus<DepositEvent>,
}

#[derive(Debug, Serialize)]
pub struct FederationMeta {
    pub picture: Option<String>,
    pub welcome: Option<String>,
    pub guardians: Vec<Guardian>,
}

#[derive(Debug, Serialize, Clone, Eq, PartialEq)]
pub struct Guardian {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Transaction {
    pub received: bool,
    pub amount: u64,
    pub module: String,
    pub timestamp: u64,
    pub operation_id: Vec<u8>,
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
pub struct DepositEvent {
    pub federation_id: FederationId,
    pub event_kind: DepositEventKind,
}

impl Multimint {
    pub async fn new(db: Database, creation_type: MultimintCreation) -> anyhow::Result<Self> {
        let mnemonic = match creation_type {
            MultimintCreation::New => {
                let mnemonic = Bip39RootSecretStrategy::<12>::random(&mut thread_rng());
                Client::store_encodable_client_secret(&db, mnemonic.to_entropy()).await?;
                mnemonic
            }
            MultimintCreation::LoadExisting => {
                let entropy = Client::load_decodable_client_secret::<Vec<u8>>(&db)
                    .await
                    .expect("Could not load existing secret");
                println!("Successfully loaded entropy from existing wallet");
                let mnemonic = Mnemonic::from_entropy(&entropy)?;
                println!("Created mnemonic from entropy");
                mnemonic
            }
            MultimintCreation::NewFromMnemonic { words } => {
                let all_words = words.join(" ");
                let mnemonic =
                    Mnemonic::parse_in_normalized(Language::English, all_words.as_str())?;
                Client::store_encodable_client_secret(&db, mnemonic.to_entropy()).await?;
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
            event_bus: EventBus::new(100, 1000),
        };

        multimint.load_clients().await?;
        multimint
            .spawn_pegin_address_watcher(pegin_address_monitor_rx)
            .await?;
        multimint.monitor_all_unused_pegin_addresses().await?;

        Ok(multimint)
    }

    async fn load_clients(&mut self) -> anyhow::Result<()> {
        println!("Loading all clients...");
        let mut dbtx = self.db.begin_transaction_nc().await;
        let configs = dbtx
            .find_by_prefix(&FederationConfigKeyPrefix)
            .await
            .collect::<BTreeMap<FederationConfigKey, FederationConfig>>()
            .await;
        for (id, config) in configs {
            let client = self
                .build_client(
                    &id.id,
                    &config.invite_code,
                    config.connector,
                    ClientType::New,
                )
                .await?;

            self.finish_active_subscriptions(&client, id.id).await;

            if client.has_pending_recoveries() {
                self.spawn_recovery_progress(client.clone());
            }

            self.clients.write().await.insert(id.id, client);
        }

        Ok(())
    }

    async fn finish_active_subscriptions(
        &self,
        client: &ClientHandleArc,
        federation_id: FederationId,
    ) {
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
                        self.spawn_await_receive(federation_id, op_id);
                        self.spawn_await_send(federation_id, op_id);
                    }
                    "mint" => {
                        // We could check what type of operation this is, but `await_ecash_reissue` and `await_ecash_send`
                        // will do that internally. So we just spawn both here and let one fail since it is the wrong
                        // operation type.
                        self.spawn_await_ecash_reissue(federation_id, op_id);
                        self.spawn_await_ecash_send(federation_id, op_id);
                    }
                    // Wallet operations are handled by the pegin monitor
                    "wallet" => {}
                    module => {
                        println!("Active operation needs to be driven to completion: {module}")
                    }
                }
            }
        }
    }

    async fn spawn_pegin_address_watcher(
        &self,
        mut monitor_rx: UnboundedReceiver<(FederationId, TweakIdx)>,
    ) -> anyhow::Result<()> {
        let event_bus_clone = self.event_bus.clone();
        let task_group_clone = self.task_group.clone();
        let clients_clone = self.clients.clone();

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

                    task_group_clone.spawn_cancellable("tweak index watcher", async move {
                        if let Err(e) =
                            Self::watch_pegin_address(fed_id, client, tweak_idx, event_bus).await
                        {
                            eprintln!("watch_pegin_address({}) failed: {:?}", tweak_idx.0, e);
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
        event_bus: EventBus<DepositEvent>,
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
                    let deposit_event = DepositEvent {
                        federation_id,
                        event_kind: DepositEventKind::Mempool(MempoolEvent {
                            amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                            txid: btc_out_point.txid.to_string(),
                        }),
                    };

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

                        let deposit_event = DepositEvent {
                            federation_id,
                            event_kind: DepositEventKind::AwaitingConfs(AwaitingConfsEvent {
                                amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                                txid: btc_out_point.txid.to_string(),
                                block_height: tx_height,
                                needed,
                            }),
                        };

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
                    let deposit_event = DepositEvent {
                        federation_id,
                        event_kind: DepositEventKind::Confirmed(ConfirmedEvent {
                            amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                            txid: btc_out_point.txid.to_string(),
                        }),
                    };

                    event_bus.publish(deposit_event).await;
                }
                DepositStateV2::Claimed {
                    btc_deposited,
                    btc_out_point,
                } => {
                    let deposit_event = DepositEvent {
                        federation_id,
                        event_kind: DepositEventKind::Claimed(ClaimedEvent {
                            amount: Amount::from_sats(btc_deposited.to_sat()).msats,
                            txid: btc_out_point.txid.to_string(),
                        }),
                    };

                    event_bus.publish(deposit_event).await;
                }
                DepositStateV2::Failed(e) => {
                    println!("deposit failed: {:?}", e);
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

                    let mut tweak_idx = TweakIdx(0);
                    while let Ok(data) = wallet_module.get_pegin_tweak_idx(tweak_idx).await {
                        if data.claimed.is_empty() {
                            // we found an allocated, unused address so we need to monitor
                            if let Err(_) = pegin_address_monitor_tx_clone.send((fed_id, tweak_idx))
                            {
                                eprintln!(
                                    "failed to monitor tweak index {:?} for fed {:?}",
                                    tweak_idx, fed_id
                                );
                            }
                        }
                        tweak_idx = tweak_idx.next();
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
        invite: String,
    ) -> anyhow::Result<(ClientHandleArc, FederationId)> {
        // Sometimes we want to get the federation meta before we've joined (i.e to show a preview).
        // In this case, we create a temprorary client and retrieve all the data
        let invite_code = InviteCode::from_str(&invite)?;
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

    // TODO: Need to add caching for this
    pub async fn get_federation_meta(
        &self,
        invite: String,
    ) -> anyhow::Result<(FederationMeta, FederationSelector)> {
        let (client, federation_id) = self.get_or_build_temp_client(invite.clone()).await?;

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
            invite_code: invite,
        };

        let meta = client.get_first_module::<fedimint_meta_client::MetaClientModule>();
        if let Ok(meta) = meta {
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

                    return Ok((
                        FederationMeta {
                            picture,
                            welcome,
                            guardians,
                        },
                        selector,
                    ));
                }
                None => {}
            }
        }

        Ok((
            FederationMeta {
                picture: None,
                welcome: None,
                guardians,
            },
            selector,
        ))
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
        /*
        if self.has_federation(&federation_id).await {
            bail!("Already joined federation")
        }
        */

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
            invite_code,
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
            invite_code: invite,
        })
    }

    // TODO: This is not currently being used, because we re-join a federation after recovery from a seed
    // However, we should probably be checking when joining a federation that we haven't already joined
    // and are currently not recovering.
    async fn _has_federation(&self, federation_id: &FederationId) -> bool {
        let mut dbtx = self.db.begin_transaction_nc().await;
        dbtx.get_value(&FederationConfigKey { id: *federation_id })
            .await
            .is_some()
    }

    async fn build_client(
        &self,
        federation_id: &FederationId,
        invite_code: &InviteCode,
        connector: Connector,
        client_type: ClientType,
    ) -> anyhow::Result<ClientHandleArc> {
        let client_db = match client_type {
            ClientType::Temporary => MemDatabase::new().into(),
            _ => self.get_client_database(&federation_id),
        };

        println!("Getting derivation secret");
        let secret = Self::derive_federation_secret(&self.mnemonic, &federation_id);
        println!("Got derivation secret");

        let mut client_builder = Client::builder(client_db).await?;
        client_builder.with_module_inits(self.modules.clone());
        client_builder.with_primary_module_kind(fedimint_mint_client::KIND);
        println!("Created client builder");

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
                    println!("Client is already initialized, opening using secret...");
                    client_builder.open(secret).await
                } else {
                    println!("Client is not initialized, downloading invite code...");
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
        self.task_group
            .spawn_cancellable("recovery progress", async move {
                let mut stream = client.subscribe_to_recovery_progress();
                while let Some((module_id, progress)) = stream.next().await {
                    println!("Module: {module_id} Progress: {progress}");
                }
            });
    }

    pub async fn wait_for_recovery(
        &mut self,
        invite_code: String,
    ) -> anyhow::Result<FederationSelector> {
        println!("Waiting for recovery...");
        let invite = InviteCode::from_str(&invite_code)?;
        let federation_id = invite.federation_id();
        let recovering_client = self
            .clients
            .read()
            .await
            .get(&federation_id)
            .expect("No federation exists")
            .clone();

        println!("Waiting for all recoveries...");
        recovering_client.wait_for_all_recoveries().await?;
        let selector = self.join_federation(invite_code, false).await?;
        let new_client = self
            .clients
            .read()
            .await
            .get(&federation_id)
            .expect("Client should be available")
            .clone();
        println!("Waiting for all active state machines...");
        new_client.wait_for_all_active_state_machines().await?;

        Ok(selector)
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
                        invite_code: config.invite_code.to_string(),
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
        println!("Amount with fees: {amount_with_fees:?}");
        println!("Amount without fees: {amount_without_fees:?}");
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
                println!("Using LNv2 for the actual invoice");
                return Ok((invoice, operation_id));
            }
        }

        println!("Using LNv1 for the actual invoice");
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
                    Ok(final_state) => println!("Receive completed: {final_state:?}"),
                    Err(e) => println!("Could not await receive {operation_id:?} {e:?}"),
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
        let (invoice, operation_id) = lnv2
            .receive(
                amount_with_fees,
                DEFAULT_EXPIRY_TIME_SECS,
                Bolt11InvoiceDescription::Direct(String::new()),
                Some(gateway),
                to_value(amount_without_fees)?,
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
                to_value(amount_without_fees)?,
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
            println!("Using LNv2 for selecting receive gateway");
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
        println!("Using LNv1 for selecting receive gateway");
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
    ) -> anyhow::Result<OperationId> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        let invoice = Bolt11Invoice::from_str(&invoice)?;

        if is_lnv2 {
            println!("Attempting to pay using LNv2...");
            if let Ok(lnv2_operation_id) =
                Self::pay_lnv2(&client, invoice.clone(), gateway.clone()).await
            {
                println!("Successfully initated LNv2 payment");
                return Ok(lnv2_operation_id);
            }
        }

        println!("Attempting to pay using LNv1...");
        let operation_id = Self::pay_lnv1(&client, invoice, gateway).await?;
        println!("Successfully initiated LNv1 payment");
        self.spawn_await_send(federation_id.clone(), operation_id.clone());
        Ok(operation_id)
    }

    async fn pay_lnv2(
        client: &ClientHandleArc,
        invoice: Bolt11Invoice,
        gateway: SafeUrl,
    ) -> anyhow::Result<OperationId> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let operation_id = lnv2.send(invoice, Some(gateway), ().into()).await?;
        Ok(operation_id)
    }

    async fn pay_lnv1(
        client: &ClientHandleArc,
        invoice: Bolt11Invoice,
        gateway_url: SafeUrl,
    ) -> anyhow::Result<OperationId> {
        let lnv1 = client.get_first_module::<LightningClientModule>()?;
        let gateways = lnv1.list_gateways().await;
        let gateway = gateways
            .iter()
            .find(|g| g.info.api == gateway_url)
            .ok_or(anyhow!("Could not find gateway"))?
            .info
            .clone();
        let outgoing_lightning_payment =
            lnv1.pay_bolt11_invoice(Some(gateway), invoice, ()).await?;
        Ok(outgoing_lightning_payment.payment_type.operation_id())
    }

    fn spawn_await_send(&self, federation_id: FederationId, operation_id: OperationId) {
        let self_copy = self.clone();
        self.task_group.spawn_cancellable("await send", async move {
            match self_copy.await_send(&federation_id, operation_id).await {
                Ok(final_state) => println!("Send completed: {final_state:?}"),
                Err(e) => println!("Could not await send {operation_id:?} {e:?}"),
            }
        });
    }

    pub async fn await_send(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<(FinalSendOperationState, String)> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();

        let send_state = match Self::await_send_lnv2(&client, operation_id).await {
            Ok(lnv2_final_state) => lnv2_final_state,
            Err(_) => Self::await_send_lnv1(&client, operation_id).await?,
        };
        Ok(send_state)
    }

    async fn await_send_lnv2(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<(FinalSendOperationState, String)> {
        let lnv2 = client.get_first_module::<fedimint_lnv2_client::LightningClientModule>()?;
        let mut updates = lnv2
            .subscribe_send_operation_state_updates(operation_id)
            .await?
            .into_stream();
        let mut final_state = FinalSendOperationState::Failure;
        let mut preimage_str = "".to_string();
        while let Some(update) = updates.next().await {
            match update {
                SendOperationState::Success(preimage) => {
                    final_state = FinalSendOperationState::Success;
                    preimage_str = preimage.consensus_encode_to_hex();
                }
                SendOperationState::Refunded => {
                    final_state = FinalSendOperationState::Refunded;
                }
                SendOperationState::Failure => {
                    final_state = FinalSendOperationState::Failure;
                }
                _ => {}
            }
        }
        Ok((final_state, preimage_str))
    }

    async fn await_send_lnv1(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<(FinalSendOperationState, String)> {
        let lnv1 = client.get_first_module::<LightningClientModule>()?;
        // First check if its an internal payment
        let mut final_state = None;
        let mut preimage_str = "".to_string();
        if let Ok(updates) = lnv1.subscribe_internal_pay(operation_id).await {
            let mut stream = updates.into_stream();
            while let Some(update) = stream.next().await {
                match update {
                    InternalPayState::Preimage(preimage) => {
                        final_state = Some(FinalSendOperationState::Success);
                        preimage_str = preimage.0.consensus_encode_to_hex();
                    }
                    InternalPayState::RefundSuccess {
                        out_points: _,
                        error: _,
                    } => {
                        final_state = Some(FinalSendOperationState::Refunded);
                    }
                    InternalPayState::FundingFailed { error: _ }
                    | InternalPayState::RefundError {
                        error_message: _,
                        error: _,
                    }
                    | InternalPayState::UnexpectedError(_) => {
                        final_state = Some(FinalSendOperationState::Failure);
                    }
                    _ => {}
                }
            }
        }

        if let Some(internal_final_state) = final_state {
            return Ok((internal_final_state, preimage_str));
        }

        // If internal fails, check if its an external payment
        let mut preimage_str = "".to_string();
        if let Ok(updates) = lnv1.subscribe_ln_pay(operation_id).await {
            let mut stream = updates.into_stream();
            while let Some(update) = stream.next().await {
                match update {
                    LnPayState::Success { preimage } => {
                        final_state = Some(FinalSendOperationState::Success);
                        preimage_str = preimage;
                    }
                    LnPayState::Refunded { gateway_error: _ } => {
                        final_state = Some(FinalSendOperationState::Refunded);
                    }
                    LnPayState::UnexpectedError { error_message: _ } => {
                        final_state = Some(FinalSendOperationState::Failure);
                    }
                    _ => {}
                }
            }
        }

        if let Some(external_final_state) = final_state {
            return Ok((external_final_state, preimage_str));
        }

        Ok((FinalSendOperationState::Failure, preimage_str))
    }

    pub async fn await_receive(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<FinalReceiveOperationState> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        let receive_state = match Self::await_receive_lnv2(&client, operation_id).await {
            Ok(lnv2_final_state) => lnv2_final_state,
            Err(_) => Self::await_receive_lnv1(&client, operation_id).await?,
        };

        Ok(receive_state)
    }

    async fn await_receive_lnv2(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<FinalReceiveOperationState> {
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
        Ok(final_state)
    }

    async fn await_receive_lnv1(
        client: &ClientHandleArc,
        operation_id: OperationId,
    ) -> anyhow::Result<FinalReceiveOperationState> {
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

        Ok(final_state)
    }

    async fn lnv1_update_gateway_cache(&self, client: &ClientHandleArc) -> anyhow::Result<()> {
        let lnv1_client = client.clone();
        self.task_group
            .spawn_cancellable("update gateway cache", async move {
                let lnv1 = lnv1_client
                    .get_first_module::<LightningClientModule>()
                    .expect("LNv1 should be present");
                match lnv1.update_gateway_cache().await {
                    Ok(_) => println!("Updated gateway cache"),
                    Err(e) => println!("Could not update gateway cache {e}"),
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

                let outcome = op_log_val.outcome::<serde_json::Value>();
                let timestamp = key
                    .creation_time
                    .duration_since(UNIX_EPOCH)
                    .expect("Cannot be before unix epoch")
                    .as_millis() as u64;

                let tx = match op_log_val.operation_module_kind() {
                    "lnv2" => {
                        if outcome.is_none() {
                            None
                        } else {
                            let meta = op_log_val.meta::<LightningOperationMeta>();
                            match meta {
                                LightningOperationMeta::Receive(receive) => Some(Transaction {
                                    received: true,
                                    amount: serde_json::from_value::<Amount>(receive.custom_meta)
                                        .expect("Could not deserialize amount")
                                        .msats,
                                    module: "lnv2".to_string(),
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                }),
                                LightningOperationMeta::Send(send) => Some(Transaction {
                                    received: false,
                                    amount: send.contract.amount.msats,
                                    module: "lnv2".to_string(),
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                }),
                            }
                        }
                    }
                    "ln" => {
                        if outcome.is_none() {
                            None
                        } else {
                            let meta =
                                op_log_val.meta::<fedimint_ln_client::LightningOperationMeta>();
                            match meta.variant {
                                LightningOperationMetaVariant::Pay(send) => Some(Transaction {
                                    received: false,
                                    amount: send
                                        .invoice
                                        .amount_milli_satoshis()
                                        .expect("Cannot pay amountless invoice"),
                                    module: "ln".to_string(),
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                }),
                                LightningOperationMetaVariant::Receive { invoice, .. } => {
                                    Some(Transaction {
                                        received: true,
                                        amount: invoice
                                            .amount_milli_satoshis()
                                            .expect("Cannot receive amountless invoice"),
                                        module: "ln".to_string(),
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                }
                                LightningOperationMetaVariant::RecurringPaymentReceive(
                                    recurring,
                                ) => Some(Transaction {
                                    received: true,
                                    amount: recurring
                                        .invoice
                                        .amount_milli_satoshis()
                                        .expect("Cannot receive amountless invoice"),
                                    module: "ln".to_string(),
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                }),
                                _ => None,
                            }
                        }
                    }
                    "mint" => {
                        let meta = op_log_val.meta::<MintOperationMeta>();
                        match meta.variant {
                            MintOperationMetaVariant::SpendOOB { oob_notes, .. } => {
                                Some(Transaction {
                                    received: false,
                                    amount: oob_notes.total_amount().msats,
                                    module: "mint".to_string(),
                                    timestamp,
                                    operation_id: key.operation_id.0.to_vec(),
                                })
                            }
                            MintOperationMetaVariant::Reissuance { .. } => {
                                if outcome.is_none() {
                                    None
                                } else {
                                    let amount: Amount = serde_json::from_value(meta.extra_meta)
                                        .expect("Could not get total amount");
                                    Some(Transaction {
                                        received: true,
                                        amount: amount.msats,
                                        module: "mint".to_string(),
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                }
                            }
                        }
                    }
                    "wallet" => {
                        let meta = op_log_val.meta::<WalletOperationMeta>();
                        let outcome = op_log_val.outcome::<DepositStateV2>();
                        match meta.variant {
                            WalletOperationMetaVariant::Deposit { .. } => {
                                if let Some(DepositStateV2::Claimed { btc_deposited, .. }) = outcome
                                {
                                    let amount = Amount::from_sats(btc_deposited.to_sat()).msats;
                                    Some(Transaction {
                                        received: true,
                                        amount,
                                        module: "wallet".to_string(),
                                        timestamp,
                                        operation_id: key.operation_id.0.to_vec(),
                                    })
                                } else {
                                    None
                                }
                            }
                            WalletOperationMetaVariant::Withdraw { .. } => None,
                            WalletOperationMetaVariant::RbfWithdraw { .. } => None,
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
            .expect("No federation exists")
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let amount = Amount::from_msats(amount_msats);
        // Default timeout after one day
        let timeout = Duration::from_secs(60 * 60 * 24);
        // TODO: Fix overspend
        let (operation_id, notes) = mint
            .spend_notes_with_selector(&SelectNotesWithAtleastAmount, amount, timeout, true, ())
            .await?;

        self.spawn_await_ecash_send(*federation_id, operation_id);

        Ok((operation_id, notes.to_string(), notes.total_amount().msats))
    }

    fn spawn_await_ecash_send(&self, federation_id: FederationId, operation_id: OperationId) {
        let self_copy = self.clone();
        self.task_group
            .spawn_cancellable("await ecash send", async move {
                match self_copy
                    .await_ecash_send(&federation_id, operation_id)
                    .await
                {
                    Ok(final_state) => println!("Ecash send completed: {final_state:?}"),
                    Err(e) => println!("Could not await receive {operation_id:?} {e:?}"),
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
            .expect("No federation exists")
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let mut updates = mint
            .subscribe_spend_notes(operation_id)
            .await?
            .into_stream();
        let mut final_state = SpendOOBState::UserCanceledFailure;
        while let Some(update) = updates.next().await {
            println!("Ecash send state: {update:?}");
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
            .expect("No federation exists")
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let notes = OOBNotes::from_str(&ecash)?;
        let total_amount = notes.total_amount();
        let operation_id = mint.reissue_external_notes(notes, total_amount).await?;
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
                    Ok(final_state) => println!("Ecash reissue completed: {final_state:?}"),
                    Err(e) => println!("Could not await receive {operation_id:?} {e:?}"),
                }
            });
    }

    pub async fn await_ecash_reissue(
        &self,
        federation_id: &FederationId,
        operation_id: OperationId,
    ) -> anyhow::Result<ReissueExternalNotesState> {
        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        let mint = client.get_first_module::<MintClientModule>()?;
        let mut updates = mint
            .subscribe_reissue_external_notes(operation_id)
            .await
            .unwrap()
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

        Ok(final_state)
    }

    /// Refund the full balance on-chain to the Mutinynet faucet.
    ///
    /// This is a temporary method that assists with development and should
    /// be removed before supporting mainnet.
    pub async fn refund(&self, federation_id: &FederationId) -> anyhow::Result<(String, u64)> {
        // hardcoded address for the Mutinynet faucet
        // https://faucet.mutinynet.com/
        let address =
            bitcoin::address::Address::from_str("tb1qd28npep0s8frcm3y7dxqajkcy2m40eysplyr9v")?;

        let client = self
            .clients
            .read()
            .await
            .get(federation_id)
            .expect("No federation exists")
            .clone();
        let wallet_module =
            client.get_first_module::<fedimint_wallet_client::WalletClientModule>()?;

        let address = address.require_network(wallet_module.get_network())?;
        let balance = bitcoin::Amount::from_sat(client.get_balance().await.msats / 1000);
        let fees = wallet_module.get_withdraw_fees(&address, balance).await?;
        let absolute_fees = fees.amount();
        let amount = balance
            .checked_sub(fees.amount())
            .context("Not enough funds to pay fees")?;

        println!("Attempting withdraw with fees: {fees:?}");

        let operation_id = wallet_module.withdraw(&address, amount, fees, ()).await?;

        let mut updates = wallet_module
            .subscribe_withdraw_updates(operation_id)
            .await?
            .into_stream();

        let (txid, fees_sat) = loop {
            let update = updates
                .next()
                .await
                .ok_or_else(|| anyhow!("Update stream ended without outcome"))?;

            println!("Withdraw state update: {:?}", update);

            match update {
                WithdrawState::Succeeded(txid) => {
                    break (txid.consensus_encode_to_hex(), absolute_fees.to_sat());
                }
                WithdrawState::Failed(e) => {
                    bail!("Withdraw failed: {e}");
                }
                WithdrawState::Created => {
                    continue;
                }
            }
        };

        Ok((txid, fees_sat))
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
        let tweak_idx = wallet_module.find_tweak_idx_by_address(address).await?;

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

    pub async fn subscribe_deposits(
        &self,
        federation_id: FederationId,
        sink: StreamSink<DepositEvent>,
    ) -> anyhow::Result<()> {
        let mut stream = self.event_bus.subscribe();

        while let Some(evt) = stream.next().await {
            if evt.federation_id == federation_id {
                if sink.add(evt).is_err() {
                    break;
                }
            }
        }

        Ok(())
    }

    pub async fn wallet_summary(&self, invite: String) -> anyhow::Result<Vec<Utxo>> {
        let (client, _) = self.get_or_build_temp_client(invite).await?;
        let wallet_module = client.get_first_module::<WalletClientModule>()?;
        let wallet_summary = wallet_module.get_wallet_summary().await?;
        let utxos = wallet_summary
            .spendable_utxos
            .into_iter()
            .map(Utxo::from)
            .collect();
        Ok(utxos)
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
