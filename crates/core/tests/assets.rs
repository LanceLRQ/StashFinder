use stashfinder_core::assets::*;

fn identity() -> Identity {
    Identity {
        layout: Layout::Gpt {
            disk: "disk-a".into(),
            partition: "part-a".into(),
        },
        filesystem: Some("NTFS".into()),
        serial: Some(Serial {
            source: "ntfs64".into(),
            value: "001234".into(),
        }),
    }
}

#[test]
fn partition_identity_is_independent_of_access_path() {
    let a = identity();
    assert_eq!(a.compare(&a), IdentityMatch::Same);
    let mut other = a.clone();
    other.layout = Layout::Gpt {
        disk: "disk-b".into(),
        partition: "part-b".into(),
    };
    assert_eq!(a.compare(&other), IdentityMatch::Different);
}

#[test]
fn missing_serial_is_not_a_reformat() {
    let a = identity();
    let mut b = a.clone();
    b.serial = None;
    assert_eq!(a.compare(&b), IdentityMatch::Uncertain);
    b.serial = Some(Serial {
        source: "ntfs64".into(),
        value: "999".into(),
    });
    assert_eq!(a.compare(&b), IdentityMatch::FilesystemChanged);
}

#[test]
fn weak_mbr_evidence_does_not_automatically_reconnect() {
    let mut a = identity();
    a.layout = Layout::Mbr {
        signature: 123,
        offset: 1048576,
        length: 99999,
        device: None,
    };
    assert_eq!(a.compare(&a), IdentityMatch::Uncertain);
    let mut other = a.clone();
    if let Layout::Mbr { signature, .. } = &mut other.layout {
        *signature = 456;
    }
    assert_eq!(a.compare(&other), IdentityMatch::Different);
    if let Layout::Mbr { device, .. } = &mut a.layout {
        *device = Some("real-device".into());
    }
    assert_eq!(a.compare(&a), IdentityMatch::Same);
}

#[test]
fn names_are_validated_at_the_boundary() {
    assert_eq!(validate_name("  Movies  ").unwrap(), "Movies");
    for name in ["", "   ", "hello\nworld", &"中".repeat(129)] {
        assert!(validate_name(name).is_err());
    }
}
