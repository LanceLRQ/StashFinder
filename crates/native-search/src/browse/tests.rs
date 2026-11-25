use super::*;
use stashfinder_core::{
    PROTOCOL_VERSION, Reply, Request, browse::Ancestor, read_frame, write_frame,
};
use stashfinder_windows::ipc::{create_pipe, current_user_sid};

fn folder(parent: Option<Vec<u8>>) -> Location {
    Location::Folder {
        name: "样例".into(),
        request: Box::new(DirectoryRequest {
            partition_id: "partition".into(),
            filesystem_id: "volume".into(),
            generation: Some("3".into()),
            revision: Some("8".into()),
            parent,
            offset: 0,
            folders_only: false,
            sort: Default::default(),
            filter: None,
        }),
        previous: vec![],
    }
}

fn root_page(generation: &str) -> Box<DirectoryPage> {
    Box::new(DirectoryPage {
        total: 0,
        filesystem_id: "volume".into(),
        generation: generation.into(),
        revision: "9".into(),
        location: vec![1],
        ancestors: vec![Ancestor {
            entry: vec![1],
            name: "\\".into(),
        }],
        entries: vec![],
        next: None,
        partial: false,
        indexed_at: None,
        needs_validation: false,
    })
}

#[test]
fn paging_uses_returned_offsets_and_pins_both_index_versions() {
    let mut browser = Browser {
        location: folder(None),
        listing: None,
        restore_search: false,
    };
    let mut first = root_page("3");
    first.total = 132;
    first.next = Some(37);
    browser.apply(Listing::Folder(first));
    browser.location = browser.page(true).unwrap();
    let Location::Folder { request, .. } = &browser.location else {
        panic!()
    };
    assert_eq!(request.offset, 37);
    assert_eq!(request.generation.as_deref(), Some("3"));
    assert_eq!(request.revision.as_deref(), Some("9"));
    let mut second = root_page("3");
    second.total = 132;
    second.next = Some(94);
    browser.apply(Listing::Folder(second));
    browser.location = browser.page(true).unwrap();
    let Location::Folder {
        request, previous, ..
    } = browser.page(false).unwrap()
    else {
        panic!()
    };
    assert_eq!(request.offset, 37);
    assert_eq!(previous, [0]);
}

#[tokio::test]
async fn refreshing_after_a_rebuild_returns_to_root_without_reading_the_old_folder_id() {
    let pipe = format!(
        r"\\.\pipe\StashFinder.browse-generation-{}",
        std::process::id()
    );
    let mut server = create_pipe(&pipe, &current_user_sid().unwrap(), true).unwrap();
    let backend = tokio::spawn(async move {
        server.connect().await.unwrap();
        let wire: Request =
            serde_json::from_slice(&read_frame(&mut server).await.unwrap()).unwrap();
        let Action::Directory(query) = wire.action else {
            panic!()
        };
        assert_eq!(query.filesystem_id, "volume");
        assert_eq!(
            (query.parent, query.generation, query.revision, query.offset),
            (None, None, None, 0)
        );
        let reply = Reply {
            version: PROTOCOL_VERSION,
            id: wire.id,
            body: Response::Directory(root_page("4")),
        };
        write_frame(&mut server, &serde_json::to_vec(&reply).unwrap())
            .await
            .unwrap();
    });
    let mut browser = Browser {
        location: folder(Some(vec![55])),
        listing: None,
        restore_search: true,
    };
    let listing = browser
        .location
        .load(&Endpoint::Direct(pipe), true)
        .await
        .unwrap();
    let (path, status, _) = browser.apply(listing);
    assert_eq!(path, "样例");
    assert!(status.contains("索引已重建，已返回分区根目录"));
    assert!(matches!(browser.up(), Some(Location::Partitions)));
    backend.await.unwrap();
}
