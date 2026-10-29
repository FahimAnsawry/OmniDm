// SPDX-License-Identifier: GPL-3.0-or-later

use jiff::civil::time;
use omnidm_core::engine::Method;
use omnidm_queue::schedule::{Days, Start};
use omnidm_store::{
    CategoryId, CategoryRule, CategorySettings, DownloadId, MAIN_QUEUE, NewDownload, PostAction,
    QueueId, QueueSettings, Schedule, Store, StoreError,
};

fn add_download(store: &mut Store, url: &str) -> DownloadId {
    store
        .insert_download(&NewDownload {
            url: url.into(),
            save_dir: r"C:\Downloads".into(),
            file_name: None,
            referrer: None,
            user_agent: None,
            headers: vec![],
            method: Method::Get,
            body: vec![],
        })
        .unwrap()
}

fn night_queue() -> QueueSettings {
    QueueSettings {
        name: "Night".into(),
        max_concurrent: 2,
        schedule: Schedule {
            start: Some(Start::Daily {
                at: time(2, 0, 0, 0),
                days: Days::WEEKDAYS,
            }),
            stop_at: Some(time(7, 0, 0, 0)),
        },
        on_complete: PostAction::Sleep,
    }
}

#[test]
fn a_new_database_has_the_main_queue() {
    let store = Store::open_in_memory().unwrap();
    let queues = store.list_queues().unwrap();
    assert_eq!(queues.len(), 1);
    assert_eq!(queues[0].id, MAIN_QUEUE);
    assert_eq!(
        queues[0].settings,
        QueueSettings {
            name: "Main".into(),
            max_concurrent: 3,
            schedule: Schedule::default(),
            on_complete: PostAction::None,
        }
    );
}

#[test]
fn a_queue_reads_back_with_its_schedule_and_action() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store.insert_queue(&night_queue()).unwrap();
    let queue = store.get_queue(id).unwrap().unwrap();
    assert_eq!(queue.settings, night_queue());

    let mut changed = night_queue();
    changed.max_concurrent = 5;
    changed.schedule = Schedule::default();
    changed.on_complete = PostAction::Shutdown;
    store.update_queue(id, &changed).unwrap();
    assert_eq!(store.get_queue(id).unwrap().unwrap().settings, changed);
    assert_eq!(store.list_queues().unwrap().len(), 2);
}

#[test]
fn queue_names_are_unique() {
    let mut store = Store::open_in_memory().unwrap();
    store.insert_queue(&night_queue()).unwrap();
    assert!(matches!(
        store.insert_queue(&night_queue()),
        Err(StoreError::Sqlite(_))
    ));
}

#[test]
fn the_main_queue_cant_be_deleted() {
    let mut store = Store::open_in_memory().unwrap();
    assert!(matches!(
        store.delete_queue(MAIN_QUEUE),
        Err(StoreError::NotAllowed(_))
    ));
    assert!(store.get_queue(MAIN_QUEUE).unwrap().is_some());
}

#[test]
fn downloads_keep_their_order_in_a_queue() {
    let mut store = Store::open_in_memory().unwrap();
    let a = add_download(&mut store, "https://e.com/a");
    let b = add_download(&mut store, "https://e.com/b");
    let c = add_download(&mut store, "https://e.com/c");
    for id in [b, a, c] {
        store.add_to_queue(MAIN_QUEUE, id).unwrap();
    }
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![b, a, c]);
    assert_eq!(store.queue_of(a).unwrap(), Some(MAIN_QUEUE));

    // Adding again keeps its place.
    store.add_to_queue(MAIN_QUEUE, b).unwrap();
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![b, a, c]);

    assert!(store.move_in_queue(c, 0).unwrap());
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![c, b, a]);
    assert!(store.move_in_queue(c, 99).unwrap());
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![b, a, c]);

    store.remove_from_queue(a).unwrap();
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![b, c]);
    assert_eq!(store.queue_of(a).unwrap(), None);
    assert!(!store.move_in_queue(a, 0).unwrap());
}

#[test]
fn a_download_is_in_one_queue_at_most() {
    let mut store = Store::open_in_memory().unwrap();
    let night = store.insert_queue(&night_queue()).unwrap();
    let a = add_download(&mut store, "https://e.com/a");
    let b = add_download(&mut store, "https://e.com/b");
    store.add_to_queue(night, b).unwrap();
    store.add_to_queue(MAIN_QUEUE, a).unwrap();
    store.add_to_queue(night, a).unwrap();
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![]);
    assert_eq!(store.queue_downloads(night).unwrap(), vec![b, a]);
    assert_eq!(store.queue_of(a).unwrap(), Some(night));
}

#[test]
fn deleting_a_queue_keeps_its_downloads() {
    let mut store = Store::open_in_memory().unwrap();
    let night = store.insert_queue(&night_queue()).unwrap();
    let a = add_download(&mut store, "https://e.com/a");
    store.add_to_queue(night, a).unwrap();
    store.delete_queue(night).unwrap();
    assert_eq!(store.get_queue(night).unwrap(), None);
    assert!(store.get_download(a).unwrap().is_some());
    assert_eq!(store.queue_of(a).unwrap(), None);
}

#[test]
fn deleting_a_download_takes_it_out_of_its_queue() {
    let mut store = Store::open_in_memory().unwrap();
    let a = add_download(&mut store, "https://e.com/a");
    store.add_to_queue(MAIN_QUEUE, a).unwrap();
    store.delete_download(a).unwrap();
    assert_eq!(store.queue_downloads(MAIN_QUEUE).unwrap(), vec![]);
}

#[test]
fn a_new_database_has_the_default_categories_without_folders() {
    let store = Store::open_in_memory().unwrap();
    let names: Vec<String> = store
        .list_categories()
        .unwrap()
        .into_iter()
        .map(|c| {
            assert_eq!(
                c.settings.save_dir, None,
                "{} has no folder yet",
                c.settings.name
            );
            c.settings.name
        })
        .collect();
    assert_eq!(
        names,
        [
            "Compressed",
            "Documents",
            "Music",
            "Pictures",
            "Programs",
            "Video"
        ]
    );
}

fn code_category() -> CategorySettings {
    CategorySettings {
        name: "Code".into(),
        rule: CategoryRule {
            hosts: vec!["github.com".into()],
            ..CategoryRule::default()
        },
        save_dir: Some(r"D:\Code".into()),
    }
}

#[test]
fn categories_are_added_last_and_can_be_moved() {
    let mut store = Store::open_in_memory().unwrap();
    let code = store.insert_category(&code_category()).unwrap();
    let ids = |store: &Store| -> Vec<CategoryId> {
        store
            .list_categories()
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(ids(&store).last(), Some(&code));
    assert_eq!(
        store.get_category(code).unwrap().unwrap().settings,
        code_category()
    );

    store.move_category(code, 0).unwrap();
    assert_eq!(ids(&store)[0], code);
    assert_eq!(
        store.list_categories().unwrap()[1].settings.name,
        "Compressed"
    );
}

#[test]
fn a_category_can_be_changed_and_deleted() {
    let mut store = Store::open_in_memory().unwrap();
    let code = store.insert_category(&code_category()).unwrap();
    let mut changed = code_category();
    changed.save_dir = None;
    changed.rule.extensions = vec!["rs".into()];
    store.update_category(code, &changed).unwrap();
    assert_eq!(store.get_category(code).unwrap().unwrap().settings, changed);

    let a = add_download(&mut store, "https://github.com/a.zip");
    store.set_download_category(a, Some(code)).unwrap();
    assert_eq!(store.get_download(a).unwrap().unwrap().category, Some(code));

    store.delete_category(code).unwrap();
    assert_eq!(store.get_category(code).unwrap(), None);
    let a = store.get_download(a).unwrap().unwrap();
    assert_eq!(a.category, None, "the download stays, with no category");
}

#[test]
fn unknown_ids_read_as_none() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.get_queue(QueueId(99)).unwrap(), None);
    assert_eq!(store.get_category(CategoryId(99)).unwrap(), None);
}
