use blockloom_core::{
    multiplayer::MultiplayerSettings,
    pack::GamePack,
    project::{self, Project},
    scene::Mode,
    wire,
};

#[test]
fn legacy_projects_remain_private_without_new_saved_fields() {
    let project = Project::starter("Private", Mode::TwoD);
    let json = serde_json::to_value(&project).unwrap();
    assert!(json.get("multiplayer").is_none());
    assert!(
        !serde_json::from_value::<Project>(json)
            .unwrap()
            .multiplayer
            .enabled
    );
    let file = serde_json::to_value(project::project_to_file(&project)).unwrap();
    assert!(file.get("multiplayer").is_none());
}

#[test]
fn opt_in_survives_wire_folder_and_pack_roundtrips() {
    let mut project = Project::starter("LAN", Mode::ThreeD);
    project.multiplayer = MultiplayerSettings {
        enabled: true,
        max_guests: 3,
    };
    let restored: Project = wire::from_wire(wire::to_wire(&project).unwrap()).unwrap();
    assert_eq!(restored.multiplayer, project.multiplayer);
    let dir = std::env::temp_dir().join(format!("blockloom-multiplayer-{}", uuid::Uuid::new_v4()));
    project::save_project(&project, &dir).unwrap();
    assert_eq!(
        project::read_project_dir(&dir).unwrap().multiplayer,
        project.multiplayer
    );
    let pack = GamePack::new(project.clone());
    pack.write(&dir.join("game.pack")).unwrap();
    assert_eq!(
        GamePack::read(&dir.join("game.pack"))
            .unwrap()
            .project
            .multiplayer,
        project.multiplayer
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_limits_fail_in_every_document_shape() {
    for value in [
        serde_json::json!({"enabled":true,"max_guests":0}),
        serde_json::json!({"max_guests":17}),
        serde_json::json!({"max_guests":-1}),
        serde_json::json!({"max_guests":2.5}),
        serde_json::json!({"max_players":4}),
    ] {
        assert!(serde_json::from_value::<MultiplayerSettings>(value.clone()).is_err());
        let mut project = serde_json::to_value(Project::starter("Bad", Mode::TwoD)).unwrap();
        project["multiplayer"] = value.clone();
        assert!(serde_json::from_value::<Project>(project).is_err());
        let mut file = serde_json::to_value(project::project_to_file(&Project::starter(
            "Bad",
            Mode::TwoD,
        )))
        .unwrap();
        file["multiplayer"] = value;
        assert!(serde_json::from_value::<project::ProjectFile>(file).is_err());
    }
}
