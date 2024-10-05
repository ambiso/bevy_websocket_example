use std::{io::ErrorKind, net::TcpStream};

use avian3d::prelude::*; // completely unnecessary but I like physics;
use bevy::{
    ecs::world::CommandQueue,
    prelude::*,
    tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task},
};
use tungstenite::{connect, http::Response, stream::MaybeTlsStream, Message, WebSocket};

fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, setup_scene)
        .add_systems(Update, check_connection_input)
        .add_systems(Update, setup_connection)
        .add_systems(Update, handle_tasks)
        .add_event::<WebSocketConnectionEvents>()
        .add_systems(Update, send_info)
        .add_systems(Update, recv_info)
        .run();
}

#[derive(Component)]
struct WebSocketClient(
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

#[derive(Component)]
struct WebSocketConnectionSetupTask(#[allow(unused)] Task<CommandQueue>);

fn setup_connection(
    mut ev_connect: EventReader<WebSocketConnectionEvents>,
    mut commands: Commands,
) {
    for ev in ev_connect.read() {
        match ev {
            WebSocketConnectionEvents::SetupConnection => {
                info!("Setting up connection!");
                let pool = AsyncComputeTaskPool::get();
                let entity = commands.spawn_empty().id();
                let task = pool.spawn(async move {
                    let client = connect("wss://echo.websocket.org/").unwrap();
                    info!("Connected successfully!");
                    let mut command_queue = CommandQueue::default();

                    command_queue.push(move |world: &mut World| {
                        world
                            .entity_mut(entity)
                            .insert(WebSocketClient(client))
                            // Task is complete, so remove task component from entity
                            .remove::<WebSocketConnectionSetupTask>();
                    });

                    command_queue
                });
                commands
                    .entity(entity)
                    .insert(WebSocketConnectionSetupTask(task));
            }
        }
    }
}

/// This system queries for entities that have our Task<Transform> component. It polls the
/// tasks to see if they're complete. If the task is complete it takes the result, adds a
/// new [`Mesh3d`] and [`MeshMaterial3d`] to the entity using the result from the task's work, and
/// removes the task component from the entity.
fn handle_tasks(mut commands: Commands, mut transform_tasks: Query<&mut WebSocketConnectionSetupTask>) {
    for mut task in &mut transform_tasks {
        if let Some(mut commands_queue) = block_on(future::poll_once(&mut task.0)) {
            // append the returned command queue to have it execute later
            commands.append(&mut commands_queue);
        }
    }
}

fn send_info(
    some_data: Query<(&Transform,)>,
    mut entities_with_client: Query<(&mut WebSocketClient,)>,
) {
    for (mut client,) in entities_with_client.iter_mut() {
        let transforms = &some_data.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
        info!("Sending data: {transforms:?}");
        client
            .0
             .0
            .send(Message::Binary(bincode::serialize(transforms).unwrap()))
            .unwrap();
    }
}

fn recv_info(mut q: Query<(&mut WebSocketClient,)>) {
    for (mut client,) in q.iter_mut() {
        match client.0 .0.read() {
            Ok(m) => info!("Received message {m:?}"),
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => { /* ignore */ }
            Err(e) => warn!("error receiving: {e}"),
        }
    }
}

/// Add some stuff to the scene so it's not super boring
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn(());

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
