//! Executable design spike: generic retained reducer over the real collection event API.
//! No server support is claimed; the transport extension is specified in the design.
#[path = "../src/notice.generated.rs"]
mod notice;
use notice::NoticeSnapshot;
use serde::de::DeserializeOwned;
use st3_client::{CollectionEvent, Resource};
use std::{collections::BTreeMap, marker::PhantomData};

trait Descriptor: DeserializeOwned {
    const REGISTERED_KIND: &'static str;
}
impl Descriptor for NoticeSnapshot {
    const REGISTERED_KIND: &'static str = "attention.notice-snapshot";
}
struct Watch<T> {
    rows: BTreeMap<String, Resource>,
    order: Vec<String>,
    fresh: bool,
    marker: PhantomData<T>,
}
fn watch<T: Descriptor>() -> Watch<T> {
    Watch { rows: BTreeMap::new(), order: Vec::new(), fresh: false, marker: PhantomData }
}
impl<T: Descriptor> Watch<T> {
    fn apply(&mut self, event: CollectionEvent) -> Result<(), &'static str> {
        match event {
            CollectionEvent::Snapshot { items, order, .. } => {
                self.rows.clear();
                self.rows.extend(items.into_iter().map(|row| (row.header().id.clone(), row)));
                self.order = order;
                self.fresh = true;
            }
            CollectionEvent::Changes { upserts, removes, order, .. } if self.fresh => {
                for id in removes { self.rows.remove(&id); }
                self.rows.extend(upserts.into_iter().map(|row| (row.header().id.clone(), row)));
                self.order = order;
            }
            CollectionEvent::Changes { .. } => return Err("changes before fresh snapshot"),
            CollectionEvent::Resync { .. } | CollectionEvent::Error { .. } => self.fresh = false,
            _ => return Err("wrong subscription event"),
        }
        Ok(())
    }
    fn current(&self) -> Result<Vec<T>, Box<dyn std::error::Error>> {
        if !self.fresh { return Err("stale watch".into()); }
        self.order.iter().map(|id| {
            let row = self.rows.get(id).ok_or("missing ordered row")?;
            // Existing Resource decoding adds header-only fields; the production descriptor
            // must decode the raw row once instead of this legacy enum round trip.
            let mut wire = serde_json::to_value(row)?;
            let object = wire.as_object_mut().ok_or("resource is not an object")?;
            object.remove("operational");
            Ok(serde_json::from_value(wire)?)
        }).collect()
    }
}
// Shape A is only a semantic convenience over the same retained layer.
fn follow_notices() -> Watch<NoticeSnapshot> { watch::<NoticeSnapshot>() }
fn replay() -> Result<(), Box<dyn std::error::Error>> {
    let frames: Vec<serde_json::Value> = serde_json::from_str(include_str!("../../../tests/notice-contract/frames.json"))?;
    let mut sidebar_a = follow_notices();
    let mut sidebar_b = watch::<NoticeSnapshot>();
    let mut tray = watch::<NoticeSnapshot>();
    assert_eq!(NoticeSnapshot::REGISTERED_KIND, "attention.notice-snapshot");
    for frame in frames {
        // CollectionEvent::from_frame is private; deserialize each field into the actual API types.
        let event = || -> Result<CollectionEvent, serde_json::Error> {
            if frame["kind"] == "snapshot" {
                Ok(CollectionEvent::Snapshot { id: "notice".into(), snapshot: serde_json::from_value(frame["snapshot"].clone())?, items: serde_json::from_value(frame["items"].clone())?, order: serde_json::from_value(frame["order"].clone())?, has_more: false })
            } else {
                Ok(CollectionEvent::Changes { id: "notice".into(), snapshot: serde_json::from_value(frame["snapshot"].clone())?, upserts: serde_json::from_value(frame["upserts"].clone())?, removes: vec![], order: serde_json::from_value(frame["order"].clone())?, has_more: false })
            }
        };
        sidebar_a.apply(event()?)?;
        sidebar_b.apply(event()?)?;
        tray.apply(event()?)?;
        assert_eq!(sidebar_a.current()?, sidebar_b.current()?);
        assert_eq!(sidebar_b.current()?, tray.current()?);
    }
    assert_eq!(tray.current()?[0].seen_at.as_deref(), Some("2026-10-19T12:00:01Z"));
    println!("A/B replay: sidebar and tray agree after seen; 2 frames, 1 retained row per watch");
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> { replay() }
