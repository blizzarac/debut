//! `debut-collab-server [--port 7878] [--editor-code C] [--reviewer-code C]
//! [project.debut]`: serve a project for collaborative editing; accepted
//! edits are saved back to the file. Codes not given are generated and
//! printed; share them with the people who should join.

use std::net::TcpListener;

fn main() {
    let mut port = 7878u16;
    let mut path: Option<String> = None;
    let mut invite = debut_collab_server::new_invite();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--editor-code" => invite.editor = args.next().unwrap_or(invite.editor),
            "--reviewer-code" => invite.reviewer = args.next().or(invite.reviewer),
            _ => path = Some(a),
        }
    }
    let project = match &path {
        Some(p) => match std::fs::read_to_string(p).map(|t| debut_project::schema::from_json(&t)) {
            Ok(Ok(project)) => project,
            _ => {
                eprintln!("cannot read project {p}");
                std::process::exit(1);
            }
        },
        None => debut_project::Project::new(debut_core::IdGen::random().fresh(), "Shared"),
    };
    let save = path.clone().map(|p| {
        Box::new(move |project: &debut_project::Project| {
            if let Ok(json) = debut_project::schema::to_json(project) {
                let tmp = format!("{p}.tmp");
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, &p);
                }
            }
        }) as debut_collab_server::OnChange
    });
    let listener = TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|e| {
        eprintln!("cannot listen on {port}: {e}");
        std::process::exit(1);
    });
    let handle =
        debut_collab_server::serve(listener, project, save, invite.clone()).expect("serve");
    println!("debut collaboration server on {}", handle.addr);
    println!("editor code:   {}", invite.editor);
    if let Some(code) = &invite.reviewer {
        println!("reviewer code: {code}");
    }
    loop {
        std::thread::park();
    }
}
