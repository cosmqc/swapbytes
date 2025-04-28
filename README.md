# SwapBytes P2P file-sharing app

Swapbytes is a peer-to-peer file sharing app with chat, direct message, and file trade features.

## Features
- Decentralized chat using Gossipsub
- Public file metadata sharing using DHT
- File share request logic, you don't have to swap files if you don't want to
- Forced swaps, meaning you will always get a file from the other party
- Private DMs for negotiations
- Peer discovery using mDNS and Kademlia
- Rendezvous server support

## Building
- If you haven't already, [install Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html)
- Clone my repository (git clone https://github.com/cosmqc/swapbytes) 
- Navigate to swapbytes/
- Run `cargo build`
- For each peer, run `cargo run --`



## Getting started
### Command-line options
- `--port <port>`: Port number to listen on, defaults to a random unused port
- `--rendezvous <ip>`: An optional rendezvous server, defaults to the local network, stops if it can't connect

For example:
```bash
cargo run -- --port 9999 --rendezvous 10.0.0.1
```

### Enter your nickname
When the app starts up, you will be asked for a nickname to identify yourself. You can always change it later using the `/nick` command

### Chatting
Any messages not prefixed by a forward slash (/) will be sent as messages. Make sure to say hello when you join!

### Commands
Any messages prefixed by a forward slash (/) will be sent as commands. The set of commands is below.

Any multiword arguments should be wrapped in double quotes. For example:
```bash
/nick "lebron james"
```
Commands are case-insensitive, but arguments are case-sensitive.

- `/help`: Show a help message.
- `/nick <nickname>`: Change your nickname.
- `/list_peers`: List all the peers currently on the network.
- `/upload <filename> <description (optional)>`: Upload a file to the application. This will share the name of the file, its size, and other metadata, but not the actual file. You are given a hash when you upload a file that you use to identify it.
- `/list_files`: Show a list of all the files that have been uploaded, grouped by the uploader.
- `/dm <nickname> <message>`: In the middle of a trade, you can DM the other trader to discuss private details about the trade.
- `/trade <nickname> <your_file_hash> <their_file_hash>`: Send a trade offer that the other trader can accept or decline. Use /list_files to find other people's file hashes.
- `/trade_accept <nickname>`: Accept a trade offer. The files will transfer immediately.
- `/trade_decline <nickname>`: Decline a trade offer.