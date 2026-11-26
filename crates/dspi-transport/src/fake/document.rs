//! The demo device seeded from a document (WP P1OF, PRO1 W7): offline editing.
//!
//! A `.dspipreset` document opened with no device becomes a [`FakeDevice`]
//! whose image *is* the document: every screen reads and edits it as it would
//! a device, and an export writes it back. The caller (the core's
//! `offline.rs`) turns the document into a V32 image and a `GET_PLATFORM`
//! reply ([`DocumentSeed::capture`]); this file only installs them, with the
//! document's name on the running preset and as the device's name, and its
//! two preset-directory modes. Nothing here touches USB.

use super::{Capture, FakeDevice, FakeOptions, MV_INDEPENDENT, NAME_LEN, OC_WITH_PRESET, put_name};

/// What a document seeds a demo device with.
#[derive(Debug, Clone)]
pub struct DocumentSeed {
    /// The `GET_PLATFORM` reply (platform and output count) and the V32 image,
    /// its header naming the document's channel counts.
    pub capture: Capture,
    /// The document's name: slot 0's preset name and the device's name, cut to
    /// what the firmware stores (31 bytes, on a character boundary).
    pub name: String,
    /// `meta.masterVolumeMode`: 0 independent, 1 with the preset.
    pub master_volume_mode: u8,
    /// `meta.outputConfigMode`: 0 independent, 1 with the preset.
    pub output_config_mode: u8,
}

/// `name` cut to at most `NAME_LEN - 1` bytes without splitting a character.
pub(super) fn stored_name(name: &str) -> &[u8] {
    let max = NAME_LEN.saturating_sub(1);
    let mut end = name.len().min(max);
    while end > 0 && !name.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    name.get(..end).unwrap_or("").as_bytes()
}

impl FakeDevice {
    /// The demo device (extension channel, control surfaces, analyser and
    /// generator, as [`FakeDevice::demo`]) holding `seed`'s image: the device a
    /// document describes. A mode other than 0 or 1 reads as the firmware
    /// default. Nothing in the seed is trusted: an odd image gives a device
    /// with that image, never a panic.
    pub fn from_document(seed: DocumentSeed) -> Self {
        let options = FakeOptions {
            extensions: true,
            surfaces: true,
            analysis: true,
            ..FakeOptions::default()
        };
        let dev = Self::new(seed.capture, options);
        {
            let mut s = dev.shared.lock();
            let name = stored_name(&seed.name);
            if !name.is_empty() {
                if let Some(slot) = s.presets.names.first_mut() {
                    put_name(slot, name);
                }
                s.extras.set_device_name(name);
            }
            s.presets.master_volume_mode = match seed.master_volume_mode {
                0 | 1 => seed.master_volume_mode,
                _ => MV_INDEPENDENT,
            };
            s.presets.output_config_mode = match seed.output_config_mode {
                0 | 1 => seed.output_config_mode,
                _ => OC_WITH_PRESET,
            };
        }
        dev
    }
}

#[cfg(test)]
mod tests_wp_p1of {
    use super::*;
    use crate::Transport;
    use dspi_proto::generated::opcodes as op;

    fn seed(name: &str) -> DocumentSeed {
        DocumentSeed {
            capture: Capture::demo(),
            name: name.into(),
            master_volume_mode: 1,
            output_config_mode: 0,
        }
    }

    #[test]
    fn wp_p1of_seeded_device_answers_from_the_seed() {
        let mut cap = Capture::demo();
        // An RP2040's shape: 2 inputs, 5 outputs.
        cap.platform[0] = 0;
        cap.platform[3] = 5;
        cap.bulk[1] = 0;
        cap.bulk[2] = 7;
        cap.bulk[3] = 5;
        cap.bulk[4] = 2;
        let mut dev = FakeDevice::from_document(DocumentSeed {
            capture: cap.clone(),
            ..seed("Hall")
        });
        let platform = dev.control_in(op::REQ_GET_PLATFORM, 0, 4).unwrap();
        assert_eq!(platform, cap.platform[..4].to_vec());
        assert_eq!(dev.remote().image(), cap.bulk);
        let s = dev.shared.lock();
        assert_eq!(&s.presets.names[0][..4], b"Hall");
        assert_eq!(s.presets.master_volume_mode, 1);
        assert_eq!(s.presets.output_config_mode, 0);
        assert_eq!(s.status_len(), 7 * 2 + 7);
    }

    #[test]
    fn wp_p1of_names_are_cut_on_a_character_boundary() {
        // 30 ASCII bytes, then a 3-byte character that would end at byte 33.
        let long = format!("{}€x", "a".repeat(30));
        assert_eq!(stored_name(&long), "a".repeat(30).as_bytes());
        assert_eq!(stored_name("Ünïcødé"), "Ünïcødé".as_bytes());
        assert_eq!(stored_name(""), b"");
        let dev = FakeDevice::from_document(seed(&long));
        let s = dev.shared.lock();
        assert_eq!(&s.presets.names[0][..30], "a".repeat(30).as_bytes());
        assert_eq!(s.presets.names[0][30], 0);
    }

    #[test]
    fn wp_p1of_odd_modes_read_as_the_defaults_and_an_empty_seed_does_not_panic() {
        let dev = FakeDevice::from_document(DocumentSeed {
            capture: Capture {
                platform: vec![],
                bulk: vec![],
            },
            name: String::new(),
            master_volume_mode: 9,
            output_config_mode: 9,
        });
        let s = dev.shared.lock();
        assert_eq!(s.presets.master_volume_mode, MV_INDEPENDENT);
        assert_eq!(s.presets.output_config_mode, OC_WITH_PRESET);
        assert_eq!(&s.presets.names[0][..4], b"Demo");
    }
}

// -- WP SHOWFIX -- (finding 12)
#[cfg(test)]
mod tests_wp_showfix {
    use crate::Transport;
    use crate::fake::FakeDevice;
    use dspi_proto::generated::opcodes as op;

    #[test]
    fn wp_showfix_a_demo_stores_the_name_it_is_given() {
        let mut d = FakeDevice::demo().with_device_name("Centre");
        let name = d.control_in(op::REQ_GET_DEVICE_NAME, 0, 32).unwrap();
        assert_eq!(name.get(..7), Some(&b"Centre\0"[..]));
    }
}
