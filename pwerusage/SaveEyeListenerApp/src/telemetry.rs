//! SaveEye telemetry payloads.
//!
//! The JSON schema is the one documented by SaveEye for local MQTT
//! (`message-schema/Local-Message-Schema.xlsx` in `saveeye/SaveEye-HA-Guide`).
//! The same schema is used by the official Home Assistant integration
//! (`saveeye/SaveEye-HomeAssistant`), whose sensor definitions inspired the
//! derived values below.
//!
//! Not every meter reports every field: a SaveEye+ for Kamstrup Omnipower may
//! only report totals and no per-phase values, so all sections are optional.

use serde::{Deserialize, Serialize};

/// Values that can be reported per phase (`L1`, `L2`, `L3`) and as a `total`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhaseValues {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(default, rename = "L1", skip_serializing_if = "Option::is_none")]
    pub l1: Option<f64>,
    #[serde(default, rename = "L2", skip_serializing_if = "Option::is_none")]
    pub l2: Option<f64>,
    #[serde(default, rename = "L3", skip_serializing_if = "Option::is_none")]
    pub l3: Option<f64>,
}

impl PhaseValues {
    /// The reported total, or the sum of the phases that are present.
    ///
    /// Some meters (including several Kamstrup models) only expose per-phase
    /// readings, while others only expose a total.
    pub fn total_or_sum(&self) -> Option<f64> {
        self.total.or_else(|| {
            let phases: Vec<f64> = [self.l1, self.l2, self.l3].into_iter().flatten().collect();
            if phases.is_empty() {
                None
            } else {
                Some(phases.iter().sum())
            }
        })
    }

    /// Arithmetic mean of the phases that are present, falling back to `total`.
    ///
    /// Useful for quantities where summing phases makes no sense, such as
    /// voltage.
    pub fn mean_or_total(&self) -> Option<f64> {
        let phases: Vec<f64> = [self.l1, self.l2, self.l3].into_iter().flatten().collect();
        if phases.is_empty() {
            self.total
        } else {
            Some(phases.iter().sum::<f64>() / phases.len() as f64)
        }
    }
}

/// An optional SaveEye "extender" (a remote pulse counter).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtenderData {
    #[serde(default)]
    pub extender_device_id: Option<String>,
    #[serde(default)]
    pub extender_pulses: Option<f64>,
    /// UNIX timestamp in seconds.
    #[serde(default)]
    pub extender_timestamp: Option<i64>,
    #[serde(default)]
    pub extender_vcc: Option<f64>,
    #[serde(default)]
    pub extender_adv_type: Option<f64>,
    #[serde(default)]
    pub extender_rssi: Option<i32>,
    /// Instantaneous active power in W.
    #[serde(default)]
    pub active_actual_consumption: Option<f64>,
    /// Cumulative active energy in Wh.
    #[serde(default)]
    pub active_total_consumption: Option<f64>,
}

/// A complete telemetry message published by a SaveEye device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    #[serde(default)]
    pub saveeye_device_serial_number: Option<String>,
    #[serde(default)]
    pub meter_type: Option<String>,
    #[serde(default)]
    pub meter_serial_number: Option<String>,
    /// ISO 8601 timestamp of the reading, as reported by the device.
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub wifi_rssi: Option<i32>,

    /// Instantaneous active power, in W.
    #[serde(default)]
    pub active_actual_consumption: Option<PhaseValues>,
    /// Instantaneous active production, in W.
    #[serde(default)]
    pub active_actual_production: Option<PhaseValues>,

    /// Cumulative active energy, in Wh.
    #[serde(default)]
    pub active_total_consumption: Option<PhaseValues>,
    /// Cumulative active production, in Wh.
    #[serde(default)]
    pub active_total_production: Option<PhaseValues>,

    /// Instantaneous reactive power, in var.
    #[serde(default)]
    pub reactive_actual_consumption: Option<PhaseValues>,
    #[serde(default)]
    pub reactive_actual_production: Option<PhaseValues>,

    /// Cumulative reactive energy, in varh.
    #[serde(default)]
    pub reactive_total_consumption: Option<PhaseValues>,
    #[serde(default)]
    pub reactive_total_production: Option<PhaseValues>,

    /// RMS voltage, in V.
    #[serde(default)]
    pub rms_voltage: Option<PhaseValues>,
    /// RMS current, in mA.
    #[serde(default)]
    pub rms_current: Option<PhaseValues>,
    /// Power factor, as a percentage.
    #[serde(default)]
    pub power_factor: Option<PhaseValues>,

    #[serde(default)]
    pub extenders_data: Option<Vec<ExtenderData>>,
}

impl Telemetry {
    /// Serial of the SaveEye device, or `"unknown"` when the payload has none.
    pub fn device_serial(&self) -> &str {
        self.saveeye_device_serial_number
            .as_deref()
            .map(str::trim)
            .filter(|serial| !serial.is_empty())
            .unwrap_or("unknown")
    }

    /// A coarse meter family, derived from `meterType`.
    pub fn meter_kind(&self) -> &'static str {
        let meter_type = self
            .meter_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if meter_type.contains("kamstrup") || meter_type.contains("omnipower") {
            "kamstrup"
        } else if meter_type.is_empty() {
            "unknown"
        } else {
            "other"
        }
    }
}

/// A stored reading: the original telemetry plus values derived from it.
///
/// The telemetry fields are flattened into the same JSON object, so consumers
/// see the original schema (`saveeyeDeviceSerialNumber`, `activeTotalConsumption`,
/// ...) alongside the derived conveniences.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// When this listener received the message (RFC 3339).
    pub received_at: String,
    /// `"kamstrup"`, `"other"` or `"unknown"`.
    pub meter_kind: String,
    /// Import minus export, in W. Positive means the site is drawing power.
    pub net_consumption_w: Option<f64>,
    /// Cumulative active consumption in kWh.
    pub active_total_consumption_kwh: Option<f64>,
    /// Cumulative active production in kWh.
    pub active_total_production_kwh: Option<f64>,

    #[serde(flatten)]
    pub telemetry: Telemetry,
}

impl Reading {
    pub fn new(telemetry: Telemetry, received_at: impl Into<String>) -> Self {
        let consumption = telemetry
            .active_actual_consumption
            .as_ref()
            .and_then(PhaseValues::total_or_sum);
        let production = telemetry
            .active_actual_production
            .as_ref()
            .and_then(PhaseValues::total_or_sum);

        let net_consumption_w = match (consumption, production) {
            (Some(consumption), Some(production)) => Some(consumption - production),
            (Some(consumption), None) => Some(consumption),
            (None, Some(production)) => Some(-production),
            (None, None) => None,
        };

        let active_total_consumption_kwh = telemetry
            .active_total_consumption
            .as_ref()
            .and_then(PhaseValues::total_or_sum)
            .map(wh_to_kwh);
        let active_total_production_kwh = telemetry
            .active_total_production
            .as_ref()
            .and_then(PhaseValues::total_or_sum)
            .map(wh_to_kwh);

        Self {
            received_at: received_at.into(),
            meter_kind: telemetry.meter_kind().to_owned(),
            net_consumption_w,
            active_total_consumption_kwh,
            active_total_production_kwh,
            telemetry,
        }
    }

    /// One-line, human-readable summary for the Spin log.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(watts) = self.net_consumption_w {
            let direction = if watts < 0.0 {
                "exporting"
            } else {
                "importing"
            };
            parts.push(format!("{direction} {:.0} W", watts.abs()));
        }
        if let Some(kwh) = self.active_total_consumption_kwh {
            parts.push(format!("{kwh:.2} kWh consumed"));
        }
        if let Some(kwh) = self.active_total_production_kwh {
            parts.push(format!("{kwh:.2} kWh produced"));
        }
        if let Some(voltage) = self
            .telemetry
            .rms_voltage
            .as_ref()
            .and_then(PhaseValues::mean_or_total)
        {
            parts.push(format!("{voltage:.0} V"));
        }

        format!(
            "SaveEye {} [{}]{}: {}",
            self.telemetry.device_serial(),
            self.meter_kind,
            self.telemetry
                .meter_serial_number
                .as_deref()
                .map(|serial| format!(" meter {serial}"))
                .unwrap_or_default(),
            if parts.is_empty() {
                "no usable values".to_owned()
            } else {
                parts.join(", ")
            }
        )
    }
}

fn wh_to_kwh(wh: f64) -> f64 {
    wh / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const KAMSTRUP: &str = include_str!("../tests/fixtures/kamstrup-telemetry.json");

    #[test]
    fn parses_a_kamstrup_payload() {
        let telemetry: Telemetry = serde_json::from_str(KAMSTRUP).expect("valid SaveEye payload");

        assert_eq!(telemetry.device_serial(), "J85LH2H4");
        assert_eq!(telemetry.meter_kind(), "kamstrup");
        assert_eq!(telemetry.meter_serial_number.as_deref(), Some("68412345"));
        assert_eq!(telemetry.wifi_rssi, Some(-58));
        assert_eq!(
            telemetry
                .active_actual_consumption
                .as_ref()
                .and_then(PhaseValues::total_or_sum),
            Some(812.0)
        );
        assert_eq!(
            telemetry.rms_voltage.as_ref().and_then(|v| v.l1),
            Some(231.0)
        );
    }

    #[test]
    fn derives_kwh_and_net_power() {
        let telemetry: Telemetry = serde_json::from_str(KAMSTRUP).unwrap();
        let reading = Reading::new(telemetry, "2026-10-27T08:15:01Z");

        assert_eq!(reading.meter_kind, "kamstrup");
        assert_eq!(reading.net_consumption_w, Some(812.0));
        assert_eq!(reading.active_total_consumption_kwh, Some(18423.3));
        assert_eq!(reading.active_total_production_kwh, Some(2150.4));
    }

    #[test]
    fn falls_back_to_the_sum_of_phases() {
        let values = PhaseValues {
            total: None,
            l1: Some(100.0),
            l2: Some(200.0),
            l3: Some(300.0),
        };
        assert_eq!(values.total_or_sum(), Some(600.0));
        assert_eq!(values.mean_or_total(), Some(200.0));
    }

    #[test]
    fn summary_reports_average_voltage() {
        let telemetry: Telemetry = serde_json::from_str(KAMSTRUP).unwrap();
        let reading = Reading::new(telemetry, "2026-10-27T08:15:01Z");

        // 231, 229 and 233 V average to 231 V - never 693 V.
        assert!(reading.summary().contains("231 V"));
    }

    #[test]
    fn net_power_is_negative_when_exporting() {
        let telemetry = Telemetry {
            active_actual_consumption: Some(PhaseValues {
                total: Some(0.0),
                ..Default::default()
            }),
            active_actual_production: Some(PhaseValues {
                total: Some(1200.0),
                ..Default::default()
            }),
            ..Default::default()
        };

        let reading = Reading::new(telemetry, "2026-10-27T12:00:00Z");
        assert_eq!(reading.net_consumption_w, Some(-1200.0));
        assert!(reading.summary().contains("exporting 1200 W"));
    }

    #[test]
    fn keeps_original_fields_when_serializing() {
        let telemetry: Telemetry = serde_json::from_str(KAMSTRUP).unwrap();
        let reading = Reading::new(telemetry, "2026-10-27T08:15:01Z");
        let json: serde_json::Value = serde_json::to_value(&reading).unwrap();

        assert_eq!(json["saveeyeDeviceSerialNumber"], "J85LH2H4");
        assert_eq!(json["meterKind"], "kamstrup");
        assert_eq!(json["netConsumptionW"], 812.0);
        assert_eq!(json["activeTotalConsumptionKwh"], 18423.3);
        assert_eq!(json["activeTotalConsumption"]["total"], 18_423_300.0);
    }

    #[test]
    fn tolerates_minimal_payloads_and_unknown_fields() {
        let telemetry: Telemetry = serde_json::from_str(
            r#"{
                "saveeyeDeviceSerialNumber": "ABC123",
                "meterType": "/LGF5E360",
                "somethingNew": {"nested": true},
                "activeTotalConsumption": {"total": 4200}
            }"#,
        )
        .unwrap();

        assert_eq!(telemetry.meter_kind(), "other");
        assert_eq!(telemetry.device_serial(), "ABC123");
        assert_eq!(
            Reading::new(telemetry, "2026-10-27T08:15:01Z").net_consumption_w,
            None
        );
    }
}
