    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Replays $CUPRITA_REPLAY (skill `look`, replay.sh): lines "frame step args…", points.
        thread_local!(static STEPS: Vec<(u32, Vec<String>)> = {
            let text = std::env::var("CUPRITA_REPLAY").ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
            let mut steps = vec![];
            for line in text.lines() {
                let w: Vec<String> = line.split_whitespace().map(String::from).collect();
                let Some(f) = w.first().and_then(|f| f.parse::<u32>().ok()) else { continue };
                let n = |i: usize| w.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                let at = |s: &str, x: f32, y: f32, m: &str| vec![s.to_string(), x.to_string(), y.to_string(), m.to_string()];
                match w.get(1).map(String::as_str) {
                    Some(click @ ("click" | "rclick")) => {
                        let (mods, r) = (w.get(4).cloned().unwrap_or_default(), if click == "rclick" { "r" } else { "" });
                        steps.extend([(f, at(&format!("{r}press"), n(2), n(3), &mods)), (f + 2, at(&format!("{r}release"), n(2), n(3), &mods))]);
                    }
                    Some("drag") => {
                        let (x0, y0, x1, y1) = (n(2), n(3), n(4), n(5));
                        let mods = w.get(6).cloned().unwrap_or_default();
                        steps.push((f, at("press", x0, y0, &mods)));
                        for k in 1..=10 {
                            let t = k as f32 / 10.0;
                            steps.push((f + k, at("move", x0 + t * (x1 - x0), y0 + t * (y1 - y0), &mods)));
                        }
                        steps.push((f + 11, at("release", x1, y1, &mods)));
                    }
                    Some(_) => steps.push((f, w[1..].to_vec())),
                    None => {}
                }
            }
            steps
        });
        let f = self.frames;
        let now: Vec<Vec<String>> = STEPS.with(|s| s.iter().filter(|(at, _)| *at == f).map(|(_, a)| a.clone()).collect());
        let mods = |m: &str| egui::Modifiers { alt: m.contains("alt"), ctrl: m.contains("ctrl"), shift: m.contains("shift"), mac_cmd: false, command: m.contains("ctrl") };
        let num = |a: &[String], i: usize| a.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        for a in now {
            let pos = egui::pos2(num(&a, 1), num(&a, 2));
            match a[0].as_str() {
                "key" => {
                    // `key ctrl+shift K`, or `key K`: a letter gets its text too, as a keyboard sends it.
                    let (m, name) = if a.len() > 2 { (mods(&a[1]), a[2].as_str()) } else { (mods(""), a[1].as_str()) };
                    let key = egui::Key::from_name(name).unwrap_or_else(|| panic!("no key {name}"));
                    raw.events.push(egui::Event::ModifiersChanged(m));
                    raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
                    if name.chars().count() == 1 && !m.ctrl && !m.alt {
                        raw.events.push(egui::Event::Text(if m.shift { name.to_uppercase() } else { name.to_lowercase() }));
                    }
                    raw.events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
                }
                "text" => raw.events.push(egui::Event::Text(a[1..].join(" "))),
                // What a real Ctrl+C or Ctrl+V gives egui: the system clipboard is written, or read
                // (`paste` hands the text it would hold).
                "copy" => raw.events.push(egui::Event::Copy),
                // `drop <file>`: the file dropped on the window, as a file manager drops it.
                "drop" => {
                    #[derive(Debug)]
                    struct Dropped(std::path::PathBuf);
                    impl egui::DroppedFile for Dropped {
                        fn path(&self) -> &std::path::Path {
                            &self.0
                        }
                        fn bytes(&self) -> Result<Vec<u8>, String> {
                            std::fs::read(&self.0).map_err(|e| e.to_string())
                        }
                    }
                    raw.dropped_files.push(std::sync::Arc::new(Dropped(a[1..].join(" ").into())));
                }
                "paste" => raw.events.push(egui::Event::Paste(a[1..].join(" "))),
                "move" => raw.events.push(egui::Event::PointerMoved(pos)),
                "press" | "release" | "rpress" | "rrelease" => {
                    let m = mods(a.get(3).map_or("", String::as_str));
                    raw.events.push(egui::Event::ModifiersChanged(m));
                    raw.events.push(egui::Event::PointerMoved(pos));
                    let (pressed, right) = (a[0].ends_with("press"), matches!(a[0].as_str(), "rpress" | "rrelease"));
                    let button = if right { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                    raw.events.push(egui::Event::PointerButton { pos, button, pressed, modifiers: m });
                }
                "scroll" => raw.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, num(&a, 1)),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                }),
                "find" => self.find(&a[1]),
                "say" => match (&self.sch, self.bench) {
                    (Some(s), Bench::Schematic) => {
                        let lit = s.lit.as_ref().map(|l| &l.0);
                        eprintln!("replay {f}: sch status={:?} picked={:?} lit={lit:?} pointer={:?}", s.status, s.picked, self.pointer)
                    }
                    _ => eprintln!(
                        "replay {f}: status={:?} bench={:?} selected={:?} picked={:?} net={:?} pointer={:?}",
                        self.status,
                        self.bench,
                        self.selected,
                        self.picked,
                        self.board.nets.get(self.net as usize).filter(|_| self.net != 0),
                        self.pointer
                    ),
                },
                "shot" => self.screenshot = Some(PathBuf::from(&a[1])),
                "snap" => {
                    let path = PathBuf::from(&a[1]);
                    ctx.request_screenshot(move |img| {
                        if let Err(e) = save_png(&path, img.size[0] as u32, img.size[1] as u32, img.as_raw()) {
                            eprintln!("{}: {e}", path.display());
                        }
                    });
                }
                other => eprintln!("replay: no step {other}"),
            }
        }
        ctx.request_repaint();
    }
