use std::error::Error;
use libp2p::gossipsub::IdentTopic;
use libp2p::kad;
use regex::Regex;
use crate::utils::{self, add_peer_to_store, prompt_for_nickname};
use crate::events::SwapBytesBehaviour;
use libp2p::swarm::Swarm;
use tokio::io::{BufReader, Stdin, Lines};


pub async fn handle_input_line(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    line: String,
    current_topic: IdentTopic,
    stdin: &mut Lines<BufReader<Stdin>>
) -> Result<(), Box<dyn Error>> {
    if line.trim().is_empty() {
        return Ok(());
    }

    // All inputs without the command prefix should just get sent as messages
    if line.chars().nth(0).unwrap() != '/' {
        swarm.behaviour_mut().chat.gossipsub.publish(
            current_topic, 
            line.as_bytes()
        ).expect("Failed to send message");
        return Ok(());
    }

    let args = split_string(&line[1..]);
    let cmd = if let Some(cmd) = args.get(0) {
        cmd
    } else {
        println!("No command given");
        return Ok(());
    };

    match cmd.to_lowercase().as_str() {
        "help" => { 
            println!("So you want help? *mr krabs playing the worlds smallest violin*");
            Ok(())
        },

        "nick" => {
            // Wrong number of arguments
            if args.len() > 3 {
                println!("Invalid number of arguments. Correct usage: /nick <nickname>. Use quotes if your nickname has multiple words.");
                return Ok(());
            }

            // If only the command provided, enter prompt loop
            let nickname = if let Some(nickname) = args.get(1) {
                nickname
            } else {
                prompt_for_nickname(stdin, swarm).await;
                return Ok(());
            };

            // If given nickname is empty, enter prompt loop
            if nickname.trim().is_empty() {
                println!("Nickname cannot be empty. Please enter a valid nickname.");
                prompt_for_nickname(stdin, swarm).await;
                return Ok(());
            }

            // Otherwise, set nickname to whatevers given
            utils::set_nickname(
                swarm,
                nickname.to_string()
            );

            Ok(())
        }

        "get_peer" => { 
            if args.len() < 2 || args[1].len() == 0 {
                println!("not enough arguments");
                return Ok(());
            }
            let title = args[1].clone();
            let key = kad::RecordKey::new(&title);
            swarm.behaviour_mut().kademlia.get_record(key);
            return Ok(());
        },

        "add_peer" => {
            if args.len() < 3 || args[1].len() == 0 {
                println!("Not enough arguments");
                return Ok(());
            }
            let _ = add_peer_to_store(&mut swarm.behaviour_mut().kademlia, args[1].clone(), args[2].clone());

            Ok(())
        },
        
        _ => {
            Ok(())
        }
    }
}

fn split_string(input: &str) -> Vec<String> {
    let re = Regex::new(r#""([^"]*)"|\S+"#).unwrap();
    re.captures_iter(input)
        .map(|cap| cap.get(0).unwrap().as_str().to_string())
        .collect()
}