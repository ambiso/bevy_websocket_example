use std::{
    io::ErrorKind,
    net::TcpStream,
    sync::{Arc, Mutex},
    time::Duration,
};

use avian3d::prelude::*; // completely unnecessary but I like physics;
use bevy::{
    ecs::world::CommandQueue,
    prelude::*,
    tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task},
};
use iyes_perf_ui::{entries::PerfUiBundle, PerfUiPlugin};

#[cfg(not(target_arch = "wasm32"))]
use tungstenite::{connect, http::Response, stream::MaybeTlsStream, Message, WebSocket};

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("Failed to install rustls crypto provider");
    }
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(PerfUiPlugin)
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin)
        .add_plugins(bevy::diagnostic::EntityCountDiagnosticsPlugin)
        .add_plugins(bevy::diagnostic::SystemInformationDiagnosticsPlugin)
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, setup_scene)
        .add_systems(Update, check_connection_input)
        .add_systems(Update, setup_connection)
        .add_systems(Update, handle_tasks)
        .add_event::<WebSocketConnectionEvents>()
        .add_systems(Update, send_info)
        .add_systems(Update, recv_info)
        .insert_resource(SendMessageConfig {
            timer: Timer::new(Duration::from_secs(1), TimerMode::Repeating),
        })
        .run();
}

#[cfg(target_arch = "wasm32")]
mod wasm_websocket {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use bevy::log::info;
    use web_sys::{
        js_sys::{ArrayBuffer, Uint8Array},
        wasm_bindgen::{prelude::Closure, JsCast},
        BinaryType, Event, MessageEvent,
    };

    pub struct Client {
        pub socket: web_sys::WebSocket,
        pub recv_queue: Rc<RefCell<VecDeque<Vec<u8>>>>,
        _open_cb: Closure<dyn FnMut(Event)>,
        _message_cb: Closure<dyn FnMut(MessageEvent)>,
    }

    impl Client {
        pub fn new(url: &str) -> send_wrapper::SendWrapper<Self> {
            info!("Opening wasm websocket");
            let recv_queue = Rc::new(RefCell::new(VecDeque::new()));
            let socket = web_sys::WebSocket::new(url).expect("Failed to create WebSocket object");
            socket.set_binary_type(BinaryType::Arraybuffer);
            let open_cb: Closure<dyn FnMut(_)> = Closure::new(|_event: Event| {
                web_sys::console::log_1(&"Connection opened".into());
            });
            socket
                .add_event_listener_with_callback("open", open_cb.as_ref().dyn_ref().unwrap())
                .unwrap();
            let message_cb: Closure<dyn FnMut(_)> = Closure::new({
                let recv_queue = Rc::clone(&recv_queue);
                move |event: MessageEvent| {
                    web_sys::console::log_1(&format!("Got message: {:?}", event.data()).into());
                    if let Some(buf) = event.data().dyn_ref::<ArrayBuffer>() {
                        recv_queue
                            .borrow_mut()
                            .push_back(Uint8Array::new(buf).to_vec());
                    }
                }
            });
            socket
                .add_event_listener_with_callback("message", message_cb.as_ref().dyn_ref().unwrap())
                .unwrap();
            send_wrapper::SendWrapper::new(Client {
                socket,
                recv_queue,
                _open_cb: open_cb,
                _message_cb: message_cb,
            })
        }
    }
}

#[derive(Component)]
struct WebSocketClient(
    #[cfg(target_arch = "wasm32")] send_wrapper::SendWrapper<wasm_websocket::Client>,
    #[cfg(not(target_arch = "wasm32"))]
    (
        WebSocket<MaybeTlsStream<TcpStream>>,
        Response<Option<Vec<u8>>>,
    ),
);

#[derive(Event)]
enum WebSocketConnectionEvents {
    SetupConnection,
}

fn check_connection_input(
    input: Res<ButtonInput<KeyCode>>,
    mut ev_connect: EventWriter<WebSocketConnectionEvents>,
) {
    if input.just_pressed(KeyCode::Space) {
        // set up connection
        ev_connect.send(WebSocketConnectionEvents::SetupConnection);
    }
}

use thiserror::Error;
use web_sys::MessageEvent;
#[cfg(target_arch = "wasm32")]
use web_sys::{
    wasm_bindgen::{prelude::Closure, JsCast, JsValue},
    Event,
};

#[derive(Error, Debug)]
enum ConnectionSetupError {
    #[error("IO")]
    Io(#[from] std::io::Error),
    #[cfg(target_arch = "wasm32")]
    #[error("WebSocket")]
    WebSocket(), // TODO: remove or fill in actual error and do error handling with it?
    #[cfg(not(target_arch = "wasm32"))]
    #[error("WebSocket")]
    WebSocket(#[from] tungstenite::Error),
}

#[derive(Component)]
struct WebSocketConnectionSetupTask(
    #[allow(unused)] Task<Result<CommandQueue, ConnectionSetupError>>,
);

fn setup_connection(
    mut ev_connect: EventReader<WebSocketConnectionEvents>,
    mut commands: Commands,
) {
    for ev in ev_connect.read() {
        match ev {
            WebSocketConnectionEvents::SetupConnection => {
                info!("Setting up connection!");
                let url = "wss://echo.websocket.org/";
                let entity = commands.spawn_empty().id();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let pool = AsyncComputeTaskPool::get();
                    let task = pool.spawn(async move {
                        let mut client = connect(url)?;
                        match client.0.get_mut() {
                            MaybeTlsStream::Plain(p) => p.set_nonblocking(true)?,
                            MaybeTlsStream::Rustls(stream_owned) => {
                                stream_owned.get_mut().set_nonblocking(true)?
                            }
                            _ => todo!(),
                        };
                        info!("Connected successfully!");
                        let mut command_queue = CommandQueue::default();

                        command_queue.push(move |world: &mut World| {
                            world
                                .entity_mut(entity)
                                .insert(WebSocketClient(client))
                                // Task is complete, so remove task component from entity
                                .remove::<WebSocketConnectionSetupTask>();
                        });

                        Ok(command_queue)
                    });
                    commands
                        .entity(entity)
                        .insert(WebSocketConnectionSetupTask(task));
                }
                #[cfg(target_arch = "wasm32")]
                {
                    commands
                        .entity(entity)
                        .insert(WebSocketClient(wasm_websocket::Client::new(url)));
                }
            }
        }
    }
}

fn handle_tasks(
    mut commands: Commands,
    mut transform_tasks: Query<&mut WebSocketConnectionSetupTask>,
) {
    for mut task in &mut transform_tasks {
        if let Some(result) = block_on(future::poll_once(&mut task.0)) {
            // append the returned command queue to have it execute later
            match result {
                Ok(mut commands_queue) => {
                    commands.append(&mut commands_queue);
                }
                Err(e) => {
                    info!("Connection failed with: {e:?}");
                }
            }
        }
    }
}

#[derive(Resource)]
struct SendMessageConfig {
    timer: Timer,
}

fn send_info(
    some_data: Query<(&Transform,)>,
    time: Res<Time>,
    mut entities_with_client: Query<(&mut WebSocketClient,)>,
    mut config: ResMut<SendMessageConfig>,
) {
    config.timer.tick(time.delta());
    if config.timer.finished() {
        // only send messages once every second, so we don't spam the server
        info!("Time to send data again...");
        for (mut client,) in entities_with_client.iter_mut() {
            let transforms = &some_data.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
            info!("Sending data: {transforms:?}");
            let msg = bincode::serialize(transforms).unwrap();
            #[cfg(target_arch = "wasm32")]
            {
                // TODO: do some handling so we know whether the websocket is connected yet
                let _ = client.0.socket.send_with_u8_array(msg.as_slice()); // ignore the error because the websocket may still be connecting
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                match client.0 .0.send(Message::Binary(msg)) {
                    Ok(_) => info!("Data successfully sent!"),
                    #[cfg(not(target_arch = "wasm32"))]
                    Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => { /* ignore */
                    }
                    Err(e) => {
                        warn!("Could not send the message: {e:?}");
                    }
                }
            }
        }
    }
}

fn recv_info(mut q: Query<(&mut WebSocketClient,)>) {
    for (mut client,) in q.iter_mut() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match client.0 .0.read() {
                Ok(m) => info!("Received message {m:?}"),
                Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => { /* ignore */
                }
                Err(e) => warn!("error receiving: {e}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            while let Some(m) = client.0.recv_queue.borrow_mut().pop_front() {
                info!("Received message {m:?}")
            }
        }
    }
}

/// Add some stuff to the scene so it's not super boring
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn(PerfUiBundle::default());

    // circular base
    commands
        .spawn(PbrBundle {
            mesh: meshes.add(Circle::new(4.0)),
            material: materials.add(Color::WHITE),
            transform: Transform::from_rotation(Quat::from_rotation_x(
                -std::f32::consts::FRAC_PI_2,
            )),
            ..default()
        })
        .insert((RigidBody::Static, Collider::half_space(Vec3::Z)));
    // cube
    commands
        .spawn(PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(124, 144, 255)),
            transform: Transform::from_xyz(0.0, 2.5, 0.0),
            ..default()
        })
        .insert((RigidBody::Dynamic, Collider::cuboid(1.0, 1.0, 1.0)));
    // light
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0),
        ..default()
    });
    // camera
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(-2.5, 4.5, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });
}
