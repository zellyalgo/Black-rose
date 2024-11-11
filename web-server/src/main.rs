use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use futures::{AsyncReadExt, SinkExt, Stream, StreamExt};
use rand::Rng;
use serde::Deserialize;
use tokio::sync::broadcast;
use axum::{
    routing::get,
    Router,
    extract::ws::{WebSocketUpgrade, WebSocket, Message},
};
use axum::extract::State;
use axum::http::Method;
use axum::response::{IntoResponse};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct Tile {
    id: usize,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GameData {
    jails: Vec<Tile>,
    tiles: Vec<Tile>,
    coordinates: Coordinates,
}

#[derive(Debug, Deserialize, Eq, Hash, PartialEq, Clone)]
#[serde(rename_all = "PascalCase")]
struct Coordinate {
    vertical: i32,
    horizontal: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Coordinates {
    players: Players,
    tier0: Vec<Coordinate>,
    tier1: Vec<Coordinate>,
    tier2: Vec<Coordinate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Players {
    player2: Vec<Coordinate>,
    player3: Vec<Coordinate>,
    player4: Vec<Coordinate>,
    player5: Vec<Coordinate>,
    player6: Vec<Coordinate>,
}

#[derive(Debug)]
struct GameConfig {
    player_number: usize,
}

fn start_game() {
    let mut file = File::open("game.json").expect("file not found");
    let mut data = String::new();
    file.read_to_string(&mut data).expect("cannot read");
    let game: GameData = serde_json::from_str(&data).expect("failed to parse game data");
    let game_config = GameConfig {
        player_number: 3,
    };

    let board = start_board(&game, &game_config);
    println!("{:?}", board);
}

fn start_board(game: &GameData, config: &GameConfig) -> HashMap<Coordinate, Tile> {
    let mut board: HashMap<Coordinate, Tile> = HashMap::new();
    let mut possible_positions_tier0 = game.coordinates.tier0.clone();
    let mut possible_positions_tier1 = game.coordinates.tier1.clone();
    let mut possible_positions_tier2 = game.coordinates.tier2.clone();
    let possible_tiles = game.tiles.clone();
    possible_tiles.into_iter().for_each(|t| {
        let mut rng = rand::thread_rng();
        match t.id {
            7 => {
                board.insert(possible_positions_tier0.pop().expect("NOT ELEMENTS for TIER 0"), t);
            },
            8 => {
                let n = rng.gen_range(0..possible_positions_tier1.len()-1);
                board.insert(possible_positions_tier1.swap_remove(n), t);
                possible_positions_tier1.remove(n);
            },
            _ => {
                if possible_positions_tier1.len() > 0 {
                    let n = rng.gen_range(0..possible_positions_tier1.len()-1);
                    board.insert(possible_positions_tier1.swap_remove(n), t);
                    possible_positions_tier1.remove(n);
                } else {
                    let n = rng.gen_range(0..possible_positions_tier2.len()-1);
                    board.insert(possible_positions_tier2.swap_remove(n), t);
                    possible_positions_tier2.remove(n);
                }
            },
        }
    });

    match config.player_number {
        2 => {
            add_random_jails(&mut board, &game.jails, &game.coordinates.players.player2)
        },
        3 => {
            add_random_jails(&mut board, &game.jails, &game.coordinates.players.player3)
        },
        4 => {
            add_random_jails(&mut board, &game.jails, &game.coordinates.players.player4)
        },
        5 => {
            add_random_jails(&mut board, &game.jails, &game.coordinates.players.player5)
        },
        6 => {
            add_random_jails(&mut board, &game.jails, &game.coordinates.players.player6)
        }
        _ => panic!("NOT ABAILABLE THIS PLAYERS SETTINGS")
    }
    board
}

fn add_random_jails (board: &mut HashMap<Coordinate, Tile>, jails: &Vec<Tile>, coordinates: &Vec<Coordinate>) {
    let mut rng = rand::thread_rng();
    let mut clonable_jails = jails.clone();
    coordinates.into_iter().for_each(|c| {
        board.insert(c.clone(), clonable_jails.swap_remove(rng.gen_range(0..coordinates.len()-1)));
    })
}
struct AppState {
    rooms: Mutex<HashMap<String, RoomState>>,
}

struct RoomState {
    users: Mutex<HashSet<String>>,
    tx: broadcast::Sender<String>,
}

impl RoomState {
    fn new() -> Self {
        Self {
            users: Mutex::new(HashSet::new()),
            tx: broadcast::channel(69).0,
        }
    }
}

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT")
        .map(|val| val.parse::<u16>())
        .unwrap_or(Ok(3000)).unwrap();
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let app_state = Arc::new(AppState {
        rooms: Mutex::new(HashMap::new())
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(vec![Method::GET]);

    let app = Router::new()
        .route("/", get(|| async { "Hello World!" }))
        .route("/ws", get(handler))
        .route("/rooms", get(get_rooms))
        .with_state(app_state)
        .layer(cors);

    println!("Hosted on {}", addr.to_string());
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handler(ws: WebSocketUpgrade,
                 State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut username = String::new();
    let mut channel = String::new();
    let mut tx = None::<broadcast::Sender<String>>;

    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(name) = msg {
            #[derive(Deserialize)]
            struct Connect {
                username: String,
                channel: String,
            }

            let connect: Connect = match serde_json::from_str(&name) {
                Ok(connect) => connect,
                Err(err) => {
                    println!("{}", &name);
                    println!("{}", err);
                    let _ = sender.send(Message::from("Failed to connect to room!")).await;
                    break;
                }
            };

            {
                let mut rooms = state.rooms.lock().unwrap();
                channel = connect.channel.clone();

                let room = rooms.entry(connect.channel).or_insert_with(RoomState::new);
                tx = Some(room.tx.clone());

                if !room.users.lock().unwrap().contains(&connect.username) {
                    room.users.lock().unwrap().insert(connect.username.to_owned());
                    username = connect.username.clone();
                }
            }

            if tx.is_some() && !username.is_empty() {
                break;
            } else {
                let _ = sender
                    .send(Message::Text(String::from("Username already taken.")))
                    .await;

                return;
            }
        }
    };

    let tx = tx.unwrap();
    let mut rx = tx.subscribe();

    let joined = format!("{} joined the chat!", username);
    let _ = tx.send(joined);

    let mut recv_messages = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    let mut send_messages = {
        let tx = tx.clone();
        let name = username.clone();
        tokio::spawn(async move {
            while let Some(Ok(Message::Text(text))) = receiver.next().await {
                let _ = tx.send(format!("{}: {}", name, text));
            }
        })
    };

    tokio::select! {
        _ = (&mut send_messages) => recv_messages.abort(),
        _ = (&mut recv_messages) => send_messages.abort(),
    }
    ;

    let left = format!("{} left the chat!", username);
    let _ = tx.send(left);
    let mut rooms = state.rooms.lock().unwrap();
    rooms.get_mut(&channel).unwrap().users.lock().unwrap().remove(&username);

    if rooms.get_mut(&channel).unwrap().users.lock().unwrap().len() == 0 {
        rooms.remove(&channel);
    }
}

async fn get_rooms(State(state): State<Arc<AppState>>) -> String {
    let rooms = state.rooms.lock().unwrap();
    let vec = rooms.keys().into_iter().collect::<Vec<&String>>();
    match vec.len() {
        0 => json!({
            "status": "No rooms found yet!",
            "rooms": []
        }).to_string(),
        _ => json!({
            "status": "Success!",
            "rooms": vec
        }).to_string()
    }
}