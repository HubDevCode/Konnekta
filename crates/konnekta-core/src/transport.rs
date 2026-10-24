//! The ways a device is reached: the network (Wi-Fi, Ethernet, Tailscale),
//! a USB cable or Bluetooth.
//!
//! A pair may choose one (synced on both devices, like permissions): it is
//! used whenever it is available. Otherwise, and when the chosen one is not
//! available, the cable beats the network and the network beats Bluetooth.
//! The computer tells the phone which ways it can reach it, since only the
//! computer drives the cable (adb) and Bluetooth (BlueZ).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Usb,
    Wifi,
    Bluetooth,
}

impl Transport {
    pub const ALL: [Transport; 3] = [Self::Usb, Self::Wifi, Self::Bluetooth];

    /// The transport of a link.
    pub fn of_link(usb: bool, bluetooth: bool) -> Self {
        if usb {
            Self::Usb
        } else if bluetooth {
            Self::Bluetooth
        } else {
            Self::Wifi
        }
    }

    /// Higher wins: the chosen transport first, then cable, network,
    /// Bluetooth.
    pub fn rank(self, preferred: Option<Transport>) -> u8 {
        if Some(self) == preferred {
            return 10;
        }
        match self {
            Self::Usb => 3,
            Self::Wifi => 2,
            Self::Bluetooth => 1,
        }
    }

    /// Bluetooth is too slow for live audio and video.
    pub fn carries_streams(self) -> bool {
        self != Self::Bluetooth
    }

    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_wins_then_cable_network_bluetooth() {
        use Transport::*;
        assert!(Usb.rank(None) > Wifi.rank(None));
        assert!(Wifi.rank(None) > Bluetooth.rank(None));
        assert!(Bluetooth.rank(Some(Bluetooth)) > Usb.rank(Some(Bluetooth)));
        assert_eq!(Transport::parse("wifi"), Some(Wifi));
        assert_eq!(Transport::parse("x"), None);
        assert_eq!(Transport::of_link(false, true), Bluetooth);
    }
}
