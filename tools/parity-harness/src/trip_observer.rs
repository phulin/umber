//! Host-side projection of full engine observations into the bounded TRIP
//! profile. JSON transport deliberately remains outside production crates.

use anyhow::{Context, Result, bail};
use tex_command::{
    CommandObservation, CommandObserver, EffectRecord, GeometryRecord,
    InputReason as CommandInputReason, InputRecord, InputTransition as CommandInputTransition,
};
use tex_oracle::{
    CanonicalValue, EffectEvent, EffectKind, Event, GeometryEvent, InputEvent, InputReason,
    InputTransition, Normalizer, ObservationStream, Tex82ObserverProfile,
};

/// Collects only the stable full-document events admitted by the TRIP
/// schema-v1 profile: committed shipouts, terminal stop, and termination.
#[derive(Default)]
pub struct TripProfileObserver {
    events: Vec<Event>,
}

impl TripProfileObserver {
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Encodes observations with the pinned oracle stream's exact header.
    ///
    /// Reusing the oracle header is valid because both sides describe the
    /// same engine profile, fixture inputs, and deterministic environment.
    pub fn canonical_json_lines(&self, oracle: &[u8]) -> Result<Vec<u8>> {
        let oracle = ObservationStream::from_canonical_json_lines(oracle)
            .context("pinned TRIP oracle stream is invalid")?;
        let mut bytes = serde_json::to_vec(&oracle.header)?;
        bytes.push(b'\n');
        let mut normalizer = Normalizer::new();
        for event in self.events.iter().cloned() {
            bytes.extend_from_slice(&serde_json::to_vec(&normalizer.normalize(event))?);
            bytes.push(b'\n');
        }
        let stream = ObservationStream::from_canonical_json_lines(&bytes)
            .context("generated Umber TRIP stream is invalid")?;
        Tex82ObserverProfile::Trip
            .validate(&stream)
            .map_err(anyhow::Error::msg)?;
        Ok(bytes)
    }
}

impl CommandObserver for TripProfileObserver {
    fn committed(&mut self, observation: CommandObservation) {
        match observation {
            CommandObservation::Input(InputRecord {
                transition: CommandInputTransition::Stop,
                reason: CommandInputReason::Source,
                ..
            }) => self.events.push(Event::Input(InputEvent {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            })),
            CommandObservation::Effect(EffectRecord {
                kind: "shipout",
                detail,
                ..
            }) => {
                let Some((channel, page)) = detail.split_once('\0') else {
                    return;
                };
                let Ok(page) = page.parse::<i64>() else {
                    return;
                };
                self.events.push(Event::Effect(EffectEvent {
                    kind: EffectKind::Shipout,
                    channel: channel.into(),
                    value: CanonicalValue::Integer(page),
                }));
            }
            CommandObservation::Effect(EffectRecord {
                kind: "terminate",
                detail,
                ..
            }) => {
                let channel = detail.strip_suffix('\0').unwrap_or(&detail);
                self.events.push(Event::Effect(EffectEvent {
                    kind: EffectKind::Terminate,
                    channel: channel.into(),
                    value: CanonicalValue::None,
                }));
            }
            _ => {}
        }
    }
}

/// Identity-separated schema-v2 geometry projection for full TRIP runs.
#[derive(Default)]
pub struct TripGeometryObserver {
    events: Vec<Event>,
}

impl TripGeometryObserver {
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn canonical_json_lines(&self, oracle: &[u8]) -> Result<Vec<u8>> {
        if self.events.is_empty() {
            bail!("TRIP geometry stream is empty");
        }
        let oracle = ObservationStream::from_canonical_json_lines(oracle)
            .context("pinned TRIP geometry oracle stream is invalid")?;
        if oracle.header.schema != 2 {
            bail!("pinned TRIP geometry oracle must use schema-v2");
        }
        let mut bytes = serde_json::to_vec(&oracle.header)?;
        bytes.push(b'\n');
        let mut normalizer = Normalizer::new();
        for event in self.events.iter().cloned() {
            bytes.extend_from_slice(&serde_json::to_vec(&normalizer.normalize(event))?);
            bytes.push(b'\n');
        }
        let stream = ObservationStream::from_canonical_json_lines(&bytes)
            .context("generated Umber TRIP geometry stream is invalid")?;
        if stream.events.is_empty()
            || stream
                .events
                .iter()
                .any(|event| !matches!(event.semantic, Event::Geometry(_)))
        {
            bail!("generated TRIP geometry stream must contain only geometry events");
        }
        Ok(bytes)
    }
}

impl CommandObserver for TripGeometryObserver {
    fn observes_geometry(&self) -> bool {
        true
    }

    fn committed(&mut self, observation: CommandObservation) {
        let CommandObservation::Geometry(record) = observation else {
            return;
        };
        let event = match record {
            GeometryRecord::Hpack {
                width_sp,
                height_sp,
                depth_sp,
            } => GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
            },
            GeometryRecord::Vpack {
                width_sp,
                height_sp,
                depth_sp,
            } => GeometryEvent::Vpack {
                width_sp,
                height_sp,
                depth_sp,
            },
            GeometryRecord::Shipout {
                page_width_sp,
                page_height_sp,
                counts,
            } => GeometryEvent::Shipout {
                page_width_sp,
                page_height_sp,
                counts,
            },
        };
        self.events.push(Event::Geometry(event));
    }
}

/// Captures the schema-v1 command profile and schema-v2 geometry profile from
/// one production execution while retaining separate stream identities.
#[derive(Default)]
pub struct TripObservers {
    pub command: TripProfileObserver,
    pub geometry: TripGeometryObserver,
}

impl CommandObserver for TripObservers {
    fn observes_geometry(&self) -> bool {
        true
    }

    fn committed(&mut self, observation: CommandObservation) {
        self.command.committed(observation.clone());
        self.geometry.committed(observation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command::{EffectRecord, InputRecord};
    use tex_oracle::ObservationHeader;

    fn oracle() -> Vec<u8> {
        let header = ObservationHeader {
            schema: 1,
            manifest: "a".repeat(64),
        };
        let mut bytes = serde_json::to_vec(&header).expect("header");
        bytes.push(b'\n');
        let mut normalizer = Normalizer::new();
        for event in [
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(1),
            }),
            Event::Input(InputEvent {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Terminate,
                channel: "engine".into(),
                value: CanonicalValue::None,
            }),
        ] {
            bytes.extend_from_slice(
                &serde_json::to_vec(&normalizer.normalize(event)).expect("event"),
            );
            bytes.push(b'\n');
        }
        bytes
    }

    #[test]
    fn bounded_profile_is_nonempty_contiguous_and_uses_pinned_header() {
        let mut observer = TripProfileObserver::default();
        observer.committed(CommandObservation::Effect(EffectRecord {
            kind: "message",
            detail: "not stable".into(),
            tokens: None,
        }));
        observer.committed(CommandObservation::Effect(EffectRecord {
            kind: "shipout",
            detail: "dvi\01".into(),
            tokens: None,
        }));
        observer.committed(CommandObservation::Input(InputRecord {
            transition: CommandInputTransition::Stop,
            reason: CommandInputReason::Source,
            source_name: None,
            level: 0,
            position: 0,
        }));
        observer.committed(CommandObservation::Effect(EffectRecord {
            kind: "terminate",
            detail: "engine\0".into(),
            tokens: None,
        }));
        let expected = oracle();
        let actual = observer
            .canonical_json_lines(&expected)
            .expect("profile encodes");
        assert_eq!(actual, expected);
        assert_eq!(observer.event_count(), 3);
    }
}
