use crossterm::{
    event::{poll, read, Event, KeyCode},
    execute,
    terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::{
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use rand::{thread_rng, Rng};
use image::GenericImageView;

// ----------------- Stations -----------------
struct Station {
    name: &'static str,
    url: &'static str,
    logo_path: &'static str,
}

const STATIONS: &[Station] = &[
    Station {
        name: "CBC Radio 1 (St. John's)",
        url: "https://cbcradiolive.akamaized.net/hls/live/2037435/ES_R1NSN/adaptive_48/chunklist_ao.m3u8",
        logo_path: "logos/CBC.png",
    },
    Station {
        name: "CHMR 93.5 FM",
        url: "http://192.99.14.49:9005/live128",
        logo_path: "logos/CHMR-FM.png",
    },
    Station {
        name: "CHOZ OZFM",
        url: "https://ozfm.streamb.live/SB00174?ver=516364",
        logo_path: "logos/CHOZ_OZFM.png",
    },
    Station {
        name: "VOCM AM 590",
        url: "https://stingray.leanstream.co/VOCMAM",
        logo_path: "logos/VOCM.png",
    },
    Station {
        name: "VOWR 800",
        url: "https://c21.radioboss.fm/stream/193",
        logo_path: "logos/VOWR.png",
    },
];

// ----------------- App State -----------------
struct AppState {
    current_station: Option<usize>,
    status: String,
    spectrum: Vec<u8>,
    spinner_index: usize,
    player: Option<Child>,
    loading_since: Option<Instant>,
}

impl AppState {
    fn new() -> Self {
        Self {
            current_station: None,
            status: "OFFLINE".to_string(),
            spectrum: vec![0; 20],
            spinner_index: 0,
            player: None,
            loading_since: None,
        }
    }
}

// ----------------- Audio -----------------
fn play(app: &mut AppState) {
    stop(app);

    if let Some(idx) = app.current_station {
        let station = &STATIONS[idx];

        app.status = "LOADING".to_string();
        app.loading_since = Some(Instant::now());

        match Command::new("ffplay")
            .arg("-nodisp")
            .arg("-loglevel")
            .arg("quiet")
            .arg(station.url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                app.player = Some(child);
            }
            Err(_) => {
                app.status = "FAILED".to_string();
                app.loading_since = None;
            }
        }
    }
}

fn stop(app: &mut AppState) {
    if let Some(ref mut p) = app.player {
        let _ = p.kill();
    }
    app.player = None;
    app.status = "OFFLINE".to_string();
}

// ----------------- Spectrum -----------------
fn update_spectrum(app: &mut AppState) {
    if let Some(start) = app.loading_since {
        if start.elapsed().as_secs_f32() > 1.5 {
            app.status = "ONLINE".to_string();
            app.loading_since = None;
        }
    }

    let mut rng = thread_rng();
    for v in &mut app.spectrum {
        if app.status == "ONLINE" {
            let delta: i8 = rng.gen_range(-2..=3);
            *v = ((*v as i8 + delta).clamp(0, 8)) as u8;
        } else {
            *v = v.saturating_sub(1);
        }
    }
    app.spinner_index = (app.spinner_index + 1) % 4;
}

// ----------------- UI -----------------
fn draw_ui(
    app: &AppState,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), std::io::Error> {
    terminal.draw(|f| {
        let size = f.size();

        // Vertical layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3),      // Title
                    Constraint::Length(3),      // Status
                    Constraint::Percentage(30), // Spectrum
                    Constraint::Min(6),         // Stations + Logo
                ]
                .as_ref(),
            )
            .split(size);

        // Title
        let title_text = match app.current_station {
            Some(idx) => format!("{} - Townie Radio", STATIONS[idx].name),
            None => "Townie Radio".to_string(),
        };
        let title = Paragraph::new(title_text)
            .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Yellow)));
        f.render_widget(title, chunks[0]);

        // Status
        let spinner = ["|", "/", "-", "\\"][app.spinner_index];
        let status = Paragraph::new(format!("STATUS: {} {}", app.status, spinner))
            .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Cyan)));
        f.render_widget(status, chunks[1]);

        // Spectrum
        let spectrum_str: String = app
            .spectrum
            .iter()
            .map(|&v| std::iter::repeat("█").take(v as usize).collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        let spectrum_widget = Paragraph::new(spectrum_str)
            .block(Block::default().borders(Borders::ALL).title("SPECTRUM").style(Style::default().fg(Color::Green)));
        f.render_widget(spectrum_widget, chunks[2]);

        // Horizontal split for Stations and Logo
        let bottom_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
            .split(chunks[3]);

        // Stations list
        let mut station_lines = vec![];
        for (i, s) in STATIONS.iter().enumerate() {
            if Some(i) == app.current_station {
                station_lines.push(format!("> [{}] {}", i + 1, s.name));
            } else {
                station_lines.push(format!("  [{}] {}", i + 1, s.name));
            }
        }
        station_lines.push("Press 1-5 to switch stations | Q = QUIT".to_string());

        let stations_paragraph = Paragraph::new(station_lines.join("\n"))
            .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Magenta)));
        f.render_widget(stations_paragraph, bottom_chunks[0]);

        // Station logo
        if let Some(idx) = app.current_station {
            if let Ok(img) = image::open(STATIONS[idx].logo_path) {
                let small = img.resize_exact(8, 8, image::imageops::FilterType::Nearest);
                let pixel_rows: Vec<Spans> = small
                    .pixels()
                    .collect::<Vec<_>>()
                    .chunks(8)
                    .map(|row: &[(u32, u32, image::Rgba<u8>)]| {
                        let spans: Vec<Span> = row
                            .iter()
                            .map(|p| {
                                let rgba = p.2 .0;
                                Span::styled("█", Style::default().fg(Color::Rgb(rgba[0], rgba[1], rgba[2])))
                            })
                            .collect();
                        Spans::from(spans)
                    })
                    .collect();
                let logo_widget = Paragraph::new(pixel_rows).block(Block::default().borders(Borders::ALL).title("LOGO"));
                f.render_widget(logo_widget, bottom_chunks[1]);
            }
        }
    })?;
    Ok(())
}

// ----------------- Main -----------------
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = Arc::new(Mutex::new(AppState::new()));

    // Spectrum animation thread
    let app_clone = Arc::clone(&app);
    thread::spawn(move || loop {
        {
            let mut a = app_clone.lock().unwrap();
            update_spectrum(&mut a);
        }
        thread::sleep(Duration::from_millis(80));
    });

    loop {
        {
            let a = app.lock().unwrap();
            draw_ui(&a, &mut terminal)?;
        }

        if poll(Duration::from_millis(50))? {
            if let Event::Key(key) = read()? {
                let mut a = app.lock().unwrap();

                match key.code {
                    KeyCode::Char('q') => {
                        stop(&mut a);
                        break;
                    }
                    KeyCode::Char(c) => {
                        if let Some(digit) = c.to_digit(10) {
                            let idx = (digit - 1) as usize;
                            if idx < STATIONS.len() {
                                stop(&mut a);
                                a.current_station = Some(idx);
                                play(&mut a);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
