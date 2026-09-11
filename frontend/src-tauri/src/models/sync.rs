use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

pub use grafyn_sync_protocol::VaultId;

pub const VAULT_DESCRIPTOR_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultDescriptorV1 {
    schema_version: u16,
    vault_id: VaultId,
}

impl VaultDescriptorV1 {
    pub fn generate() -> Self {
        let vault_id = VaultId::parse_str(&Uuid::new_v4().hyphenated().to_string())
            .expect("UUID v4 is a canonical non-nil vault id");
        Self {
            schema_version: VAULT_DESCRIPTOR_SCHEMA_VERSION,
            vault_id,
        }
    }

    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }
}

impl Serialize for VaultDescriptorV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("VaultDescriptorV1", 2)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("vault_id", &self.vault_id.to_string())?;
        state.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultDescriptorWire {
    schema_version: u16,
    vault_id: String,
}

impl<'de> Deserialize<'de> for VaultDescriptorV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = VaultDescriptorWire::deserialize(deserializer)?;
        if wire.schema_version != VAULT_DESCRIPTOR_SCHEMA_VERSION {
            return Err(D::Error::custom(format!(
                "unsupported vault descriptor schema version: {}",
                wire.schema_version
            )));
        }
        let vault_id = VaultId::parse_str(&wire.vault_id).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: wire.schema_version,
            vault_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_rejects_unknown_fields() {
        let error = serde_json::from_str::<VaultDescriptorV1>(
            r#"{"schema_version":1,"vault_id":"123e4567-e89b-42d3-a456-426614174000","extra":true}"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn descriptor_rejects_unsupported_schema_versions() {
        let error = serde_json::from_str::<VaultDescriptorV1>(
            r#"{"schema_version":2,"vault_id":"123e4567-e89b-42d3-a456-426614174000"}"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("schema version"));
    }

    #[test]
    fn vault_id_rejects_noncanonical_or_nil_values() {
        for invalid in [
            "123E4567-E89B-42D3-A456-426614174000",
            "123e4567e89b42d3a456426614174000",
            "00000000-0000-0000-0000-000000000000",
        ] {
            assert!(VaultId::parse_str(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn descriptor_serializes_a_canonical_schema_one_uuid() {
        let descriptor = VaultDescriptorV1::generate();
        let json = serde_json::to_value(&descriptor).unwrap();
        let id = json["vault_id"].as_str().unwrap();

        assert_eq!(json["schema_version"], 1);
        assert_eq!(id, descriptor.vault_id().to_string());
        assert_eq!(id, id.to_lowercase());
        assert_ne!(id, "00000000-0000-0000-0000-000000000000");
        assert_eq!(
            serde_json::from_value::<VaultDescriptorV1>(json).unwrap(),
            descriptor
        );
    }
}
