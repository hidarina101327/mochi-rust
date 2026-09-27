use super::*;
use crate::ai::{
    models::{AiToolCall, AiToolFunction},
    permission::{AiPermissionLevel, AiPermissionService},
    tools::{host::HeadlessHost, ToolRegistry},
};

#[test]
fn canvas_tools_draw_through_registry_and_honor_permissions_and_paths() {
    let root = std::env::temp_dir().join(format!(
        "mochi-canvas-tools-{}",
        crate::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("art.mcanvas");
    std::fs::write(
        &path,
        crate::canvas::serialize(&crate::canvas::CanvasDocument::empty()).unwrap(),
    )
    .unwrap();
    let permissions = Arc::new(AiPermissionService::new(&root));
    let registry = ToolRegistry::new(permissions.clone()).with(Arc::new(CanvasToolExecutor::new(
        Arc::new(HeadlessHost::new(&root)),
    )));
    let call = |name: &str, args: Value| -> Value {
        serde_json::from_str(&registry.execute(&AiToolCall {
            id: "canvas-test".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: name.into(),
                arguments: args.to_string(),
            },
        }))
        .unwrap()
    };
    let read = call(GET, json!({"path":"art.mcanvas"}));
    assert_eq!(read["ok"], true);
    let args = json!({"path":"art.mcanvas","expectedRevision":read["data"]["revision"],"drawing":{"texts":[{"text":"你好，画布","x":20,"y":30}]}});
    for level in [
        AiPermissionLevel::ReadOnly,
        AiPermissionLevel::Invisible,
        AiPermissionLevel::Suggest,
    ] {
        permissions
            .set_folder_permission(&root.to_string_lossy(), level)
            .unwrap();
        assert_eq!(call(DRAW, args.clone())["ok"], false, "{level:?}");
        assert!(
            crate::canvas::parse(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .texts
                .is_empty()
        );
    }
    permissions
        .set_folder_permission(&root.to_string_lossy(), AiPermissionLevel::Modify)
        .unwrap();
    assert_eq!(call(DRAW, args.clone())["ok"], true);
    assert_eq!(call(DRAW, args)["ok"], false); // 重试过期请求不能重复执行操作。
    assert_eq!(call(GET, json!({"path":"../outside.mcanvas"}))["ok"], false);
    std::fs::write(
        root.join("flow.mcanvas"),
        r#"{"format":"mochi.workflow-canvas","version":1,"nodes":[]}"#,
    )
    .unwrap();
    assert_eq!(call(GET, json!({"path":"flow.mcanvas"}))["ok"], false);
    assert!(registry.handles(DRAW));
    drop(registry);
    std::fs::remove_dir_all(&root).unwrap();
}
