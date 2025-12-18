// main.rs - Enhanced Screen Casting Receiver Backend
// Handles multiple client connections, device management, and web interface
use base64::Engine;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use warp::Filter;
use futures_util::{SinkExt, StreamExt};

// Device info structure (matches sender)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceInfo {
    name: String,
    width: u32,
    height: u32,
    version: String,
    capabilities: Vec<String>,
}

// Frame header structure (matches sender)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrameHeader {
    timestamp: u64,
    frame_id: u64,
    compressed_size: u32,
    original_size: u32,
    quality: u8,
}

// Client connection status
#[derive(Debug, Clone, Serialize)]
struct ClientStatus {
    device_info: DeviceInfo,
    connected_at: u64,
    last_frame_at: u64,
    total_frames: u64,
    total_bytes: u64,
    current_fps: f64,
    is_online: bool,
    client_ip: String,
}

// Frame data for streaming
#[derive(Debug, Clone)]
struct FrameData {
    header: FrameHeader,
    data: Vec<u8>,
    received_at: Instant,
}

// Shared state for all clients
type ClientMap = Arc<Mutex<HashMap<String, ClientStatus>>>;
type FrameMap = Arc<Mutex<HashMap<String, FrameData>>>;

// WebSocket message types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum WebSocketMessage {
    #[serde(rename = "device_list")]
    DeviceList { devices: Vec<ClientStatus> },
    #[serde(rename = "frame")]
    Frame {
        device_id: String,
        frame_data: String, // Base64 encoded
        header: FrameHeader,
    },
    #[serde(rename = "binary_frame")]
    BinaryFrame {
        device_id: String,
        header_id: u64,  // Change from u32 to u64 to match frame_id
    },
    #[serde(rename = "device_status")]
    DeviceStatus {
        device_id: String,
        status: ClientStatus,
    },
}

// Configuration
struct ServerConfig {
    tcp_port: u16,
    web_port: u16,
    cleanup_interval: Duration,
    offline_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            tcp_port: 6082,
            web_port: 4325,
            cleanup_interval: Duration::from_secs(5),
            offline_timeout: Duration::from_secs(10),
        }
    }
}

fn main() -> Result<()> {

     let mut res = winres::WindowsResource::new();
    res.set_icon("Admin.ico"); // path to your .ico file
    res.compile().unwrap();
    println!("🚀 Screen Casting Receiver Server v1.0.0");
    
    let config = ServerConfig::default();
    
    // Shared state
    let clients: ClientMap = Arc::new(Mutex::new(HashMap::new()));
    let frames: FrameMap = Arc::new(Mutex::new(HashMap::new()));
    
    // Broadcast channel for real-time updates
    let (tx, _rx) = broadcast::channel::<WebSocketMessage>(1000);
    let broadcast_tx = Arc::new(tx);
    
    // Start TCP server for client connections
    start_tcp_server(config.tcp_port, clients.clone(), frames.clone(), broadcast_tx.clone())?;
    
    // Start cleanup task
    start_cleanup_task(clients.clone(), config.cleanup_interval, config.offline_timeout, broadcast_tx.clone());
    
    // Start web server
    start_web_server(config.web_port, clients, frames, broadcast_tx)?;
    
    Ok(())
}

fn start_tcp_server(
    port: u16,
    clients: ClientMap,
    frames: FrameMap,
    broadcast_tx: Arc<broadcast::Sender<WebSocketMessage>>,
) -> Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))?;
    println!("🔌 TCP server listening on port {}", port);
    
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let clients = clients.clone();
                    let frames = frames.clone();
                    let broadcast_tx = broadcast_tx.clone();
                    
                    thread::spawn(move || {
                        if let Err(e) = handle_client_connection(stream, clients, frames, broadcast_tx) {
                            eprintln!("❌ Client handling error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("⚠️  Connection error: {}", e);
                }
            }
        }
    });
    
    Ok(())
}

fn handle_client_connection(
    mut stream: TcpStream,
    clients: ClientMap,
    frames: FrameMap,
    broadcast_tx: Arc<broadcast::Sender<WebSocketMessage>>,
) -> Result<()> {
    let client_addr = stream.peer_addr()?;
    println!("🔗 New client connected: {}", client_addr);
    
    // Set read timeout
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    
    let mut reader = BufReader::new(&stream);
    
    // Read protocol header
    let mut protocol_header = [0u8; 4];
    reader.read_exact(&mut protocol_header)?;
    
    if &protocol_header != b"STRM" {
        return Err(anyhow::anyhow!("Invalid protocol header"));
    }
    
    // Read protocol version
    let mut version_bytes = [0u8; 4];
    reader.read_exact(&mut version_bytes)?;
    let _version = u32::from_le_bytes(version_bytes);
    
    // Read device info size
    let mut size_bytes = [0u8; 4];
    reader.read_exact(&mut size_bytes)?;
    let info_size = u32::from_le_bytes(size_bytes);
    
    // Read device info
    let mut info_buffer = vec![0u8; info_size as usize];
    reader.read_exact(&mut info_buffer)?;
    
    let device_info: DeviceInfo = serde_json::from_slice(&info_buffer)?;
    let device_id = format!("{}_{}", device_info.name, client_addr.ip());
    
    println!("📱 Device registered: {} ({}x{})", device_id, device_info.width, device_info.height);
    
    // Add client to registry
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let client_status = ClientStatus {
        device_info: device_info.clone(),
        connected_at: now,
        last_frame_at: now,
        total_frames: 0,
        total_bytes: 0,
        current_fps: 0.0,
        is_online: true,
        client_ip: client_addr.ip().to_string(),
    };
    
    {
        let mut clients_lock = clients.lock().unwrap();
        clients_lock.insert(device_id.clone(), client_status.clone());
    }
    
    // Broadcast device list update
    broadcast_device_list(&clients, &broadcast_tx);
    
    // Frame processing variables
    let mut frame_times = std::collections::VecDeque::new();
    let mut total_frames = 0u64;
    let mut total_bytes = 0u64;
    
    // Main frame receiving loop
    loop {
        // Read frame header size
        let mut header_size_bytes = [0u8; 4];
        match reader.read_exact(&mut header_size_bytes) {
            Ok(_) => {},
            Err(_) => break, // Client disconnected
        }
        
        let header_size = u32::from_le_bytes(header_size_bytes);
        if header_size > 10240 { // Sanity check: max 10KB header
            break;
        }
        
        // Read frame header
        let mut header_buffer = vec![0u8; header_size as usize];
        if reader.read_exact(&mut header_buffer).is_err() {
            break;
        }
        
        let frame_header: FrameHeader = match serde_json::from_slice(&header_buffer) {
            Ok(header) => header,
            Err(_) => break,
        };
        
        // Read frame data
        let mut frame_data = vec![0u8; frame_header.compressed_size as usize];
        if reader.read_exact(&mut frame_data).is_err() {
            break;
        }
        
        let now = Instant::now();
        total_frames += 1;
        total_bytes += frame_data.len() as u64;
        
        // Calculate FPS
        frame_times.push_back(now);
        while frame_times.len() > 60 { // Keep last 60 frame times
            frame_times.pop_front();
        }
        
        let current_fps = if frame_times.len() > 1 {
            let duration = now.duration_since(frame_times[0]).as_secs_f64();
            if duration > 0.0 {
                (frame_times.len() - 1) as f64 / duration
            } else {
                0.0
            }
        } else {
            0.0
        };
        
        // Update client status
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        {
            let mut clients_lock = clients.lock().unwrap();
            if let Some(client) = clients_lock.get_mut(&device_id) {
                client.last_frame_at = timestamp;
                client.total_frames = total_frames;
                client.total_bytes = total_bytes;
                client.current_fps = current_fps;
                client.is_online = true;
            }
        }
        
        // Store latest frame
        let frame_data_obj = FrameData {
            header: frame_header.clone(),
            data: frame_data.clone(),
            received_at: now,
        };
        
        {
            let mut frames_lock = frames.lock().unwrap();
            frames_lock.insert(device_id.clone(), frame_data_obj);
        }
        
        // Broadcast frame to WebSocket clients (if any are subscribed)
        // First send the header as JSON
        let ws_message = WebSocketMessage::Frame {
            device_id: device_id.clone(),
            frame_data: String::new(), // Empty string as we'll send binary separately
            header: frame_header.clone(),
        };
        
        // Don't block if no receivers
        let _ = broadcast_tx.send(ws_message);
        
        // Then send the binary data directly
        let binary_message = WebSocketMessage::BinaryFrame {
            device_id: device_id.clone(),
            header_id: frame_header.frame_id, // Use frame_id instead of id
        };
        let _ = broadcast_tx.send(binary_message);
        
        // Print stats every 60 frames
        if total_frames % 60 == 0 {
            let mb_received = total_bytes as f64 / (1024.0 * 1024.0);
            println!(
                "📊 {}: {} frames, {:.1} MB, {:.1} fps",
                device_id, total_frames, mb_received, current_fps
            );
        }
    }
    
    // Client disconnected
    println!("🔌 Client disconnected: {}", device_id);
    {
        let mut clients_lock = clients.lock().unwrap();
        if let Some(client) = clients_lock.get_mut(&device_id) {
            client.is_online = false;
        }
    }
    
    // Broadcast updated device list
    broadcast_device_list(&clients, &broadcast_tx);
    
    Ok(())
}

fn start_cleanup_task(
    clients: ClientMap,
    interval: Duration,
    timeout: Duration,
    broadcast_tx: Arc<broadcast::Sender<WebSocketMessage>>,
) {
    thread::spawn(move || {
        loop {
            thread::sleep(interval);
            
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let mut needs_update = false;
            
            {
                let mut clients_lock = clients.lock().unwrap();
                for (_, client) in clients_lock.iter_mut() {
                    if client.is_online && (now - client.last_frame_at) > timeout.as_secs() {
                        client.is_online = false;
                        needs_update = true;
                        println!("⏰ Device marked offline: {}", client.device_info.name);
                    }
                }
            }
            
            if needs_update {
                broadcast_device_list(&clients, &broadcast_tx);
            }
        }
    });
}

fn broadcast_device_list(
    clients: &ClientMap,
    broadcast_tx: &Arc<broadcast::Sender<WebSocketMessage>>,
) {
    let devices: Vec<ClientStatus> = {
        let clients_lock = clients.lock().unwrap();
        clients_lock.values().cloned().collect()
    };
    
    let message = WebSocketMessage::DeviceList { devices };
    let _ = broadcast_tx.send(message);
}

#[tokio::main]
async fn start_web_server(
    port: u16,
    clients: ClientMap,
    frames: FrameMap,
    broadcast_tx: Arc<broadcast::Sender<WebSocketMessage>>,
) -> Result<()> {
    println!("🌐 Web server starting on port {}", port);
    
    // Static files route
    let static_files = warp::path("static").and(warp::fs::dir("static"));
    
    // Root route - serve dashboard
    let dashboard = warp::path::end().and(warp::fs::file("static/dashboard.html"));
    
    // API route - get device list
    let clients_for_api = clients.clone();
    let api_devices = warp::path!("api" / "devices")
        .and(warp::get())
        .map(move || {
            let clients_lock = clients_for_api.lock().unwrap();
            let devices: Vec<ClientStatus> = clients_lock.values().cloned().collect();
            warp::reply::json(&devices)
        });
    
    // API route - get latest frame for device
    let frames_for_api = frames.clone();
    let api_frame = warp::path!("api" / "frame" / String)
        .and(warp::get())
        .map(move |device_id: String| {
            let frames_lock = frames_for_api.lock().unwrap();
            if let Some(frame) = frames_lock.get(&device_id) {
                let frame_base64 = base64::engine::general_purpose::STANDARD.encode(&frame.data);
                let response = serde_json::json!({
                    "success": true,
                    "frame_data": frame_base64,
                    "header": frame.header
                });
                warp::reply::json(&response)
            } else {
                let response = serde_json::json!({
                    "success": false,
                    "error": "Frame not found"
                });
                warp::reply::json(&response)
            }
        });
    
    // WebSocket route for real-time updates
    let broadcast_for_ws = broadcast_tx.clone();
    let frames_for_ws = frames.clone();
    let websocket = warp::path("ws")
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| {
            let broadcast_rx = broadcast_for_ws.subscribe();
            let frames = frames_for_ws.clone();
            ws.on_upgrade(move |websocket| handle_websocket(websocket, broadcast_rx, frames))
        });
    
    let routes = static_files
        .or(dashboard)
        .or(api_devices)
        .or(api_frame)
        .or(websocket)
        .with(warp::cors().allow_any_origin());
    
    println!("✅ Web server ready at http://0.0.0.0:{}", port);
    println!("📋 Dashboard available at http://localhost:{}", port);
    
    warp::serve(routes)
        .run(([0, 0, 0, 0], port))
        .await;
    
    Ok(())
}

async fn handle_websocket(
    websocket: warp::ws::WebSocket,
    mut broadcast_rx: broadcast::Receiver<WebSocketMessage>,
    frames: FrameMap,
) {
    let (mut ws_sender, mut ws_receiver) = websocket.split();
    
    // Forward broadcast messages to WebSocket
    let mut sender_task = tokio::spawn(async move {
        while let Ok(message) = broadcast_rx.recv().await {
            match &message {
                WebSocketMessage::BinaryFrame { device_id, header_id } => {
                    // Get the frame data from the frames map
                    let frame_data = {
                        let frames_lock = frames.lock().unwrap();
                        if let Some(frame) = frames_lock.get(device_id) {
                            if frame.header.frame_id == *header_id {  // Use frame_id instead of id
                                frame.data.clone()
                            } else {
                                continue; // Skip if header ID doesn't match
                            }
                        } else {
                            continue; // Skip if device not found
                        }
                    };
                    
                    // Send binary data
                    let ws_message = warp::ws::Message::binary(frame_data);
                    if ws_sender.send(ws_message).await.is_err() {
                        break;
                    }
                },
                _ => {
                    // Send JSON for other message types
                    let json = serde_json::to_string(&message).unwrap();
                    let ws_message = warp::ws::Message::text(json);
                    
                    if ws_sender.send(ws_message).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    
    // Handle incoming WebSocket messages (for future client requests)
    let mut receiver_task = tokio::spawn(async move {
        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(_msg) => {
                    // Handle client requests if needed
                }
                Err(_) => break,
            }
        }
    });
    
    // Wait for either task to complete
    tokio::select! {
        _ = &mut sender_task => {
            receiver_task.abort();
        }
        _ = &mut receiver_task => {
            sender_task.abort();
        }
    }
}