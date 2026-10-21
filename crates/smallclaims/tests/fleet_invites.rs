//! What a sponsor answers each join request: the reason a refused request records, and when the
//! join route closes. The sponsor's own clock decides expiry, so these tests set it per thread.
use smallclaims::{
    fleet::{
        MemberKey,
        code::JoinCode,
        handshake::{JoinRequest, Joiner, WriterHead},
    },
    store::{
        CreatedFleetInvite, FLEET_INVITE_FAILURE_LIMIT, FleetRedemption, Store, now_ms,
        runtime::Plain, set_thread_clock,
    },
};
use std::{sync::Arc, time::Duration};

const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

/// A listening anchor that can sponsor invites.
fn sponsor() -> Store {
    let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(key.public()).unwrap();
    store.set_member_key(Some(key.clone())).unwrap();
    store
        .admit_fleet_anchor(FLEET, key.public(), "listening")
        .unwrap();
    store
}

fn invite(store: &Store, name: Option<&str>, migrate: bool) -> CreatedFleetInvite {
    store
        .create_fleet_invite(
            name,
            Duration::from_secs(60),
            &["loopback".into()],
            "person/sample",
            migrate,
        )
        .unwrap()
}

fn code(invite: &CreatedFleetInvite, migrate: bool) -> JoinCode {
    JoinCode {
        fleet_id: FLEET.parse().unwrap(),
        invite: hex::decode(&invite.invite).unwrap().try_into().unwrap(),
        token: invite.token,
        fingerprint: [0; 16],
        expires_at: invite.expires_at_unix_ms / 1000,
        name: None,
        migrate,
        endpoints: Vec::new(),
    }
}

fn request(code: &JoinCode, key: &MemberKey, name: &str, mode: &str) -> JoinRequest {
    request_from(code, key, name, mode, None)
}

/// A request from a store whose own history as `name` ends at `head`.
fn request_from(
    code: &JoinCode,
    key: &MemberKey,
    name: &str,
    mode: &str,
    head: Option<WriterHead>,
) -> JoinRequest {
    Joiner::start(code, key, name, mode, head, "test-build")
        .unwrap()
        .request()
        .clone()
}

fn joiner() -> MemberKey {
    MemberKey::generate().unwrap().0
}

fn refused(reason: &'static str) -> FleetRedemption {
    FleetRedemption::Refused(reason)
}

/// Resets this thread's clock even when an assertion fails.
struct Clock;

impl Clock {
    fn at(at: u128) -> Self {
        set_thread_clock(Some(at));
        Self
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        set_thread_clock(None);
    }
}

#[test]
fn a_node_without_an_open_invite_has_no_join_route() {
    let store = sponsor();
    let other = sponsor();
    let created = invite(&other, None, false);
    let request = request(&code(&created, false), &joiner(), "birch", "listening");
    assert_eq!(
        store.redeem_fleet_invite(&request).unwrap(),
        FleetRedemption::Closed
    );
}

#[test]
fn a_request_for_an_invite_this_node_does_not_hold_is_refused_as_unknown() {
    let store = sponsor();
    let _open = invite(&store, None, false);
    let other = sponsor();
    let elsewhere = invite(&other, None, false);
    let request = request(&code(&elsewhere, false), &joiner(), "birch", "listening");
    assert_eq!(
        store.redeem_fleet_invite(&request).unwrap(),
        refused("unknown-invite")
    );
}

#[test]
fn an_expired_invite_is_unknown_and_the_route_closes_when_none_is_left() {
    let start = now_ms();
    let _clock = Clock::at(start);
    let store = sponsor();
    let short = store
        .create_fleet_invite(
            None,
            Duration::from_secs(10),
            &["loopback".into()],
            "person/sample",
            false,
        )
        .unwrap();
    let _long = invite(&store, None, false);
    let request = request(&code(&short, false), &joiner(), "birch", "listening");

    // The sponsor's clock reaches the expiry: the token is erased, the other invite stays open.
    set_thread_clock(Some(u128::from(short.expires_at_unix_ms)));
    assert_eq!(
        store.redeem_fleet_invite(&request).unwrap(),
        refused("unknown-invite")
    );
    assert!(store.has_open_fleet_invites().unwrap());

    set_thread_clock(Some(start + 61_000));
    assert!(!store.has_open_fleet_invites().unwrap());
    assert_eq!(
        store.redeem_fleet_invite(&request).unwrap(),
        FleetRedemption::Closed
    );
}

#[test]
fn a_revoked_invite_is_unknown() {
    let store = sponsor();
    let created = invite(&store, None, false);
    let _other = invite(&store, None, false);
    store
        .revoke_fleet_invite(&created.invite, "sample", Some("person/sample"))
        .unwrap();
    let request = request(&code(&created, false), &joiner(), "birch", "listening");
    assert_eq!(
        store.redeem_fleet_invite(&request).unwrap(),
        refused("unknown-invite")
    );
}

#[test]
fn a_wrong_token_is_refused_and_enough_of_them_revoke_the_invite() {
    let store = sponsor();
    let created = invite(&store, None, false);
    let mut guessed = code(&created, false);
    guessed.token = [7; 16];
    let key = joiner();
    let guess = request(&guessed, &key, "birch", "listening");
    for _ in 1..FLEET_INVITE_FAILURE_LIMIT {
        assert_eq!(store.redeem_fleet_invite(&guess).unwrap(), refused("proof"));
        assert!(store.has_open_fleet_invites().unwrap());
    }
    assert_eq!(store.redeem_fleet_invite(&guess).unwrap(), refused("proof"));
    let views = store.fleet_invites(true).unwrap();
    assert!(
        format!("{views:?}").contains("too-many-failures"),
        "the invite was not revoked: {views:?}"
    );
    // The right token no longer helps.
    let right = request(&code(&created, false), &key, "birch", "listening");
    assert_eq!(
        store.redeem_fleet_invite(&right).unwrap(),
        FleetRedemption::Closed
    );
}

#[test]
fn a_bad_protocol_or_signature_is_refused_without_counting_as_a_guess() {
    let store = sponsor();
    let created = invite(&store, None, false);
    let key = joiner();
    let good = request(&code(&created, false), &key, "birch", "listening");
    let mut protocol = good.clone();
    protocol.protocol = "another-protocol".into();
    let mut signature = good.clone();
    signature.signature = request(&code(&created, false), &joiner(), "birch", "listening").signature;
    for _ in 0..FLEET_INVITE_FAILURE_LIMIT {
        assert_eq!(
            store.redeem_fleet_invite(&protocol).unwrap(),
            refused("request")
        );
        assert_eq!(
            store.redeem_fleet_invite(&signature).unwrap(),
            refused("request")
        );
    }
    assert!(matches!(
        store.redeem_fleet_invite(&good).unwrap(),
        FleetRedemption::Admitted { first: true, .. }
    ));
}

#[test]
fn a_join_code_used_as_a_migration_or_the_reverse_is_refused() {
    let store = sponsor();
    let join = invite(&store, None, false);
    let migration = invite(&store, None, true);
    assert_eq!(
        store
            .redeem_fleet_invite(&request(&code(&join, true), &joiner(), "birch", "listening"))
            .unwrap(),
        refused("kind")
    );
    assert_eq!(
        store
            .redeem_fleet_invite(&request(
                &code(&migration, false),
                &joiner(),
                "birch",
                "listening"
            ))
            .unwrap(),
        refused("kind")
    );
}

#[test]
fn a_name_the_invite_does_not_allow_or_an_unknown_mode_is_refused() {
    let store = sponsor();
    let pinned = invite(&store, Some("birch"), false);
    let open = invite(&store, None, false);
    for (created, name, mode) in [
        (&pinned, "cedar", "listening"),
        (&open, "-birch", "listening"),
        (&open, "local", "listening"),
        (&open, "birch", "sideways"),
    ] {
        assert_eq!(
            store
                .redeem_fleet_invite(&request(&code(created, false), &joiner(), name, mode))
                .unwrap(),
            refused("name"),
            "{name} {mode}"
        );
    }
    assert!(matches!(
        store
            .redeem_fleet_invite(&request(&code(&pinned, false), &joiner(), "birch", "dial-out"))
            .unwrap(),
        FleetRedemption::Admitted { first: true, .. }
    ));
}

#[test]
fn a_current_members_name_is_refused() {
    let store = sponsor();
    let created = invite(&store, None, false);
    assert_eq!(
        store
            .redeem_fleet_invite(&request(&code(&created, false), &joiner(), "alder", "listening"))
            .unwrap(),
        refused("name-in-use")
    );
}

#[test]
fn a_redeemed_invite_admits_only_the_same_joiner_again() {
    let store = sponsor();
    let created = invite(&store, None, false);
    let key = joiner();
    let first = request(&code(&created, false), &key, "birch", "listening");
    let FleetRedemption::Admitted {
        admitted_claim,
        first: true,
        ..
    } = store.redeem_fleet_invite(&first).unwrap()
    else {
        panic!("the first request was not admitted");
    };
    // The same joiner retrying after a lost answer gets the same admission, not a new one.
    let again = request(&code(&created, false), &key, "birch", "listening");
    assert_eq!(
        store.redeem_fleet_invite(&again).unwrap(),
        FleetRedemption::Admitted {
            token: created.token,
            writer_floor: None,
            admitted_claim,
            first: false,
        }
    );
    for (key, name) in [(&joiner(), "birch"), (&key, "cedar")] {
        assert_eq!(
            store
                .redeem_fleet_invite(&request(&code(&created, false), key, name, "listening"))
                .unwrap(),
            refused("bound-to-another-key"),
            "{name}"
        );
    }
}

#[test]
fn a_removed_name_rejoins_only_from_a_store_that_never_wrote_as_it() {
    let store = sponsor();
    let created = invite(&store, None, false);
    assert!(matches!(
        store
            .redeem_fleet_invite(&request(&code(&created, false), &joiner(), "birch", "listening"))
            .unwrap(),
        FleetRedemption::Admitted { .. }
    ));
    store
        .remove_fleet_member("birch", "sample", "person/sample")
        .unwrap();

    let migration = invite(&store, None, true);
    assert_eq!(
        store
            .redeem_fleet_invite(&request(
                &code(&migration, true),
                &joiner(),
                "birch",
                "listening"
            ))
            .unwrap(),
        refused("name-was-removed")
    );
    let rejoin = invite(&store, None, false);
    let wrote = WriterHead {
        sequence: 3,
        hash: "sample".into(),
    };
    assert_eq!(
        store
            .redeem_fleet_invite(&request_from(
                &code(&rejoin, false),
                &joiner(),
                "birch",
                "listening",
                Some(wrote)
            ))
            .unwrap(),
        refused("store-already-wrote-as-name")
    );
    // A fresh store gets a new window above everything known of the old one.
    assert!(matches!(
        store
            .redeem_fleet_invite(&request(&code(&rejoin, false), &joiner(), "birch", "listening"))
            .unwrap(),
        FleetRedemption::Admitted {
            writer_floor: Some(_),
            first: true,
            ..
        }
    ));
}
