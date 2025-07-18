use std::time::SystemTime;

use fedimint_api_client::api::net::Connector;
use fedimint_core::{
    config::{ClientConfig, FederationId},
    encoding::{Decodable, Encodable},
    impl_db_lookup, impl_db_record,
    util::SafeUrl,
};
use serde::{Deserialize, Serialize};

use crate::multimint::FederationMeta;

#[repr(u8)]
#[derive(Clone, Debug)]
pub(crate) enum DbKeyPrefix {
    FederationConfig = 0x00,
    ClientDatabase = 0x01,
    SeedPhraseAck = 0x02,
    NWC = 0x03,
    FederationMeta = 0x04,
    BtcPrice = 0x05,
    NostrRelays = 0x06,
    LightningAddress = 0x07,
    Display = 0x08,
}

#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub(crate) struct FederationConfigKey {
    pub(crate) id: FederationId,
}

#[derive(Debug, Clone, Eq, PartialEq, Encodable, Decodable, Serialize, Deserialize)]
pub(crate) struct FederationConfig {
    pub connector: Connector,
    pub federation_name: String,
    pub network: Option<String>,
    pub client_config: ClientConfig,
}

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct FederationConfigKeyPrefix;

impl_db_record!(
    key = FederationConfigKey,
    value = FederationConfig,
    db_prefix = DbKeyPrefix::FederationConfig,
);

impl_db_lookup!(
    key = FederationConfigKey,
    query_prefix = FederationConfigKeyPrefix
);

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct SeedPhraseAckKey;

impl_db_record!(
    key = SeedPhraseAckKey,
    value = (),
    db_prefix = DbKeyPrefix::SeedPhraseAck,
);

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct NostrWalletConnectKey {
    pub(crate) federation_id: FederationId,
}

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct NostrWalletConnectKeyPrefix;

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct NostrWalletConnectConfig {
    pub(crate) secret_key: [u8; 32],
    pub(crate) relay: String,
}

impl_db_record!(
    key = NostrWalletConnectKey,
    value = NostrWalletConnectConfig,
    db_prefix = DbKeyPrefix::NWC,
);

impl_db_lookup!(
    key = NostrWalletConnectKey,
    query_prefix = NostrWalletConnectKeyPrefix,
);

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct FederationMetaKey {
    pub(crate) federation_id: FederationId,
}

impl_db_record!(
    key = FederationMetaKey,
    value = FederationMeta,
    db_prefix = DbKeyPrefix::FederationMeta,
);

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct BtcPriceKey;

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct BtcPrice {
    pub(crate) price: u64,
    pub(crate) last_updated: SystemTime,
}

impl_db_record!(
    key = BtcPriceKey,
    value = BtcPrice,
    db_prefix = DbKeyPrefix::BtcPrice,
);

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct NostrRelaysKey {
    pub uri: String,
}

#[derive(Debug, Encodable, Decodable)]
pub(crate) struct NostrRelaysKeyPrefix;

impl_db_record!(
    key = NostrRelaysKey,
    value = SystemTime,
    db_prefix = DbKeyPrefix::NostrRelays,
);

impl_db_lookup!(key = NostrRelaysKey, query_prefix = NostrRelaysKeyPrefix,);

#[derive(Debug, Encodable, Decodable)]
pub struct LightningAddressKey {
    pub federation_id: FederationId,
}

#[derive(Debug, Encodable, Decodable)]
pub struct LightningAddressKeyPrefix;

#[derive(Debug, Clone, Encodable, Decodable, Serialize)]
pub struct LightningAddressConfig {
    pub username: String,
    pub domain: String,
    pub recurringd_api: SafeUrl,
    pub ln_address_api: SafeUrl,
    pub lnurl: String,
    pub authentication_token: String,
}

impl_db_record!(
    key = LightningAddressKey,
    value = LightningAddressConfig,
    db_prefix = DbKeyPrefix::LightningAddress,
);

impl_db_lookup!(
    key = LightningAddressKey,
    query_prefix = LightningAddressKeyPrefix,
);

#[derive(Debug, Clone, Encodable, Decodable, Serialize)]
pub enum DisplaySetting {
    Bip177,
    Sats,
    Nothing,
}

#[derive(Debug, Encodable, Decodable)]
pub struct DisplaySettingKey;

impl_db_record!(
    key = DisplaySettingKey,
    value = DisplaySetting,
    db_prefix = DbKeyPrefix::Display,
);
