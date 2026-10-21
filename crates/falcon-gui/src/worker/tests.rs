use std::sync::mpsc;
use std::time::{Duration, Instant};

use falcon_core::protocol::cmd;
use falcon_core::{
    DeviceEvent, Effect, KeyAssignment, Lighting, PacketDir, Profile, Rgb, layout::FACTORY_KEYMAP,
};

use super::*;

/// A simulated worker driven synchronously (no thread), plus its update stream.
fn sim_worker() -> (Worker, Receiver<Update>, Sender<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (tx, rx) = mpsc::channel();
    let mut w = Worker::new(Mode::Simulated, cmd_rx, tx, Box::new(|| {}));
    w.connect();
    (w, rx, cmd_tx)
}

fn drain(rx: &Receiver<Update>) -> Vec<Update> {
    rx.try_iter().collect()
}

fn done_results(updates: &[Update]) -> Vec<(Job, Result<(), String>)> {
    updates
        .iter()
        .filter_map(|u| match u {
            Update::Done { job, result } => Some((job.clone(), result.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn connect_reports_simulator_and_active_profile() {
    let (_w, rx, _) = sim_worker();
    let ups = drain(&rx);
    assert!(ups.iter().any(|u| matches!(
        u,
        Update::Connection(Connection::Connected {
            simulated: true,
            // The 264-byte 0x80 is not implemented, as on hardware.
            firmware: None,
            ..
        })
    )));
    assert!(ups.iter().any(|u| matches!(u, Update::ActiveProfile(0))));
    // 0x80 and 0x81 were logged.
    assert!(
        ups.iter()
            .any(|u| matches!(u, Update::Packets(p) if !p.is_empty()))
    );
}

#[test]
fn refresh_reports_the_active_layer() {
    let (mut w, rx, _) = sim_worker();
    w.sim
        .lock()
        .set_feature(&falcon_core::protocol::select_profile_request(2).unwrap())
        .unwrap();
    drain(&rx);
    w.handle(Command::Refresh);
    let ups = drain(&rx);
    assert!(ups.iter().any(|u| matches!(u, Update::ActiveProfile(2))));
    assert_eq!(done_results(&ups), vec![(Job::Refresh, Ok(()))]);
}

#[test]
fn select_profile_reaches_the_simulator() {
    let (mut w, rx, _) = sim_worker();
    drain(&rx);
    w.handle(Command::SelectProfile(3));
    assert_eq!(w.sim.lock().active_profile(), 3);
    let ups = drain(&rx);
    assert!(ups.iter().any(|u| matches!(u, Update::ActiveProfile(3))));
    assert_eq!(done_results(&ups), vec![(Job::SelectProfile(3), Ok(()))]);
}

#[test]
fn apply_lighting_and_write_profiles() {
    let (mut w, rx, _) = sim_worker();
    drain(&rx);
    let mut l = Lighting::default_for_profile(1);
    l.effect = Effect::Breathing;
    l.brightness = 2;
    w.handle(Command::ApplyLighting {
        profile: 1,
        lighting: Box::new(l),
    });
    assert_eq!(&w.sim.lock().block(1)[125..127], &[1, 1]);
    assert_eq!(w.sim.lock().commit_count(), 0);

    let mut p = Profile::default_for(2);
    p.keys[0] = KeyAssignment::Key(0x04);
    p.lighting.color = Rgb::new(1, 2, 3);
    w.handle(Command::WriteProfiles(vec![
        (2, p.clone()),
        (4, Profile::default_for(4)),
    ]));
    let sim = w.sim.lock();
    assert_eq!(sim.block(2)[0], 0x04);
    assert_eq!(&sim.block(4)[..25], &FACTORY_KEYMAP);
    assert_eq!(sim.commit_count(), 2);
    drop(sim);

    let ups = drain(&rx);
    let written: Vec<usize> = ups
        .iter()
        .filter_map(|u| match u {
            Update::Written(p, _) => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(written, vec![2, 4]);
    let packets: Vec<u8> = ups
        .iter()
        .filter_map(|u| match u {
            Update::Packets(p) => Some(p),
            _ => None,
        })
        .flatten()
        .filter(|r| r.dir == PacketDir::SetFeature)
        .map(|r| r.data[1])
        .collect();
    assert!(packets.contains(&cmd::COMMIT));
    assert_eq!(
        done_results(&ups),
        vec![
            (Job::ApplyLighting(1), Ok(())),
            (Job::Write(vec![2, 4]), Ok(()))
        ]
    );
}

#[test]
fn invalid_profile_is_reported_not_sent() {
    let (mut w, rx, _) = sim_worker();
    drain(&rx);
    let mut p = Profile::default_for(0);
    p.keys[1] = KeyAssignment::Combo(vec![]);
    w.handle(Command::WriteProfiles(vec![(0, p)]));
    let ups = drain(&rx);
    let done = done_results(&ups);
    assert_eq!(done.len(), 1);
    assert!(done[0].1.as_ref().unwrap_err().contains("layer 1"));
    assert!(!ups.iter().any(|u| matches!(u, Update::Written(..))));
    assert_eq!(w.sim.lock().commit_count(), 0);
}

#[test]
fn simulated_device_events_arrive() {
    let (mut w, rx, _) = sim_worker();
    drain(&rx);
    w.handle(Command::Simulate(SimAction::PressLayer(2)));
    w.handle(Command::Simulate(SimAction::PressHostKey(5)));
    w.poll_events();
    let ups = drain(&rx);
    let events: Vec<DeviceEvent> = ups
        .iter()
        .filter_map(|u| match u {
            Update::Event(r) => r.event,
            _ => None,
        })
        .collect();
    assert_eq!(
        events,
        vec![
            DeviceEvent::ProfileChanged { profile: 2 },
            DeviceEvent::HostKey { hw: 6 },
        ]
    );
    assert!(ups.iter().any(|u| matches!(u, Update::ActiveProfile(2))));
    assert_eq!(w.sim.lock().active_profile(), 2);
}

#[test]
fn threaded_handle_round_trip() {
    let h = WorkerHandle::spawn(Mode::Simulated, || {});
    h.send(Command::SelectProfile(1));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while Instant::now() < deadline {
        got.extend(h.try_iter());
        if !done_results(&got).is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(done_results(&got), vec![(Job::SelectProfile(1), Ok(()))]);
    drop(h); // joins the thread
}

#[test]
fn simulator_actions_need_simulated_mode() {
    let (mut w, rx, _) = sim_worker();
    w.mode = Mode::Hardware;
    w.handle(Command::Simulate(SimAction::DemoEvents));
    let done = done_results(&drain(&rx));
    assert!(done[0].1.is_err());
}
