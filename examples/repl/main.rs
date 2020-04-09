//! This is a CLI program and a little testing frame.  This file must not be
//! included when using Delta Chat Core as a library.
//!
//! Usage:  cargo run --example repl --release -- <databasefile>
//! All further options can be set using the set-command (type ? for help).

#[macro_use]
extern crate deltachat;
#[macro_use]
extern crate failure;

use std::borrow::Cow::{self, Borrowed, Owned};
use std::io::{self, Write};
use std::process::Command;

use ansi_term::Color;
use async_std::path::Path;
use deltachat::chat::ChatId;
use deltachat::config;
use deltachat::context::*;
use deltachat::oauth2::*;
use deltachat::securejoin::*;
use deltachat::Event;
use log::{error, info, warn};
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::config::OutputStreamType;
use rustyline::error::ReadlineError;
use rustyline::highlight::{Highlighter, MatchingBracketHighlighter};
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::{
    Cmd, CompletionType, Config, Context as RustyContext, EditMode, Editor, Helper, KeyPress,
};

mod cmdline;
use self::cmdline::*;

/// Event Handler
fn receive_event(event: Event) {
    let yellow = Color::Yellow.normal();
    match event {
        Event::Info(msg) => {
            /* do not show the event as this would fill the screen */
            info!("{}", msg);
        }
        Event::SmtpConnected(msg) => {
            info!("[SMTP_CONNECTED] {}", msg);
        }
        Event::ImapConnected(msg) => {
            info!("[IMAP_CONNECTED] {}", msg);
        }
        Event::SmtpMessageSent(msg) => {
            info!("[SMTP_MESSAGE_SENT] {}", msg);
        }
        Event::Warning(msg) => {
            warn!("{}", msg);
        }
        Event::Error(msg) => {
            error!("{}", msg);
        }
        Event::ErrorNetwork(msg) => {
            error!("[NETWORK] msg={}", msg);
        }
        Event::ErrorSelfNotInGroup(msg) => {
            error!("[SELF_NOT_IN_GROUP] {}", msg);
        }
        Event::MsgsChanged { chat_id, msg_id } => {
            info!(
                "{}",
                yellow.paint(format!(
                    "Received MSGS_CHANGED(chat_id={}, msg_id={})",
                    chat_id, msg_id,
                ))
            );
        }
        Event::ContactsChanged(_) => {
            info!("{}", yellow.paint("Received CONTACTS_CHANGED()"));
        }
        Event::LocationChanged(contact) => {
            info!(
                "{}",
                yellow.paint(format!("Received LOCATION_CHANGED(contact={:?})", contact))
            );
        }
        Event::ConfigureProgress(progress) => {
            info!(
                "{}",
                yellow.paint(format!("Received CONFIGURE_PROGRESS({} ‰)", progress))
            );
        }
        Event::ImexProgress(progress) => {
            info!(
                "{}",
                yellow.paint(format!("Received IMEX_PROGRESS({} ‰)", progress))
            );
        }
        Event::ImexFileWritten(file) => {
            info!(
                "{}",
                yellow.paint(format!("Received IMEX_FILE_WRITTEN({})", file.display()))
            );
        }
        Event::ChatModified(chat) => {
            info!(
                "{}",
                yellow.paint(format!("Received CHAT_MODIFIED({})", chat))
            );
        }
        _ => {
            info!("Received {:?}", event);
        }
    }
}

// === The main loop

struct DcHelper {
    completer: FilenameCompleter,
    highlighter: MatchingBracketHighlighter,
    hinter: HistoryHinter,
}

impl Completer for DcHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &RustyContext<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        self.completer.complete(line, pos, ctx)
    }
}

const IMEX_COMMANDS: [&str; 12] = [
    "initiate-key-transfer",
    "get-setupcodebegin",
    "continue-key-transfer",
    "has-backup",
    "export-backup",
    "import-backup",
    "export-keys",
    "import-keys",
    "export-setup",
    "poke",
    "reset",
    "stop",
];

const DB_COMMANDS: [&str; 11] = [
    "info",
    "open",
    "close",
    "set",
    "get",
    "oauth2",
    "configure",
    "connect",
    "disconnect",
    "maybenetwork",
    "housekeeping",
];

const CHAT_COMMANDS: [&str; 26] = [
    "listchats",
    "listarchived",
    "chat",
    "createchat",
    "createchatbymsg",
    "creategroup",
    "createverified",
    "addmember",
    "removemember",
    "groupname",
    "groupimage",
    "chatinfo",
    "sendlocations",
    "setlocation",
    "dellocations",
    "getlocations",
    "send",
    "sendimage",
    "sendfile",
    "draft",
    "listmedia",
    "archive",
    "unarchive",
    "pin",
    "unpin",
    "delchat",
];
const MESSAGE_COMMANDS: [&str; 8] = [
    "listmsgs",
    "msginfo",
    "listfresh",
    "forward",
    "markseen",
    "star",
    "unstar",
    "delmsg",
];
const CONTACT_COMMANDS: [&str; 6] = [
    "listcontacts",
    "listverified",
    "addcontact",
    "contactinfo",
    "delcontact",
    "cleanupcontacts",
];
const MISC_COMMANDS: [&str; 10] = [
    "getqr",
    "getbadqr",
    "checkqr",
    "event",
    "fileinfo",
    "clear",
    "exit",
    "quit",
    "help",
    "estimatedeletion",
];

impl Hinter for DcHelper {
    fn hint(&self, line: &str, pos: usize, ctx: &RustyContext<'_>) -> Option<String> {
        if !line.is_empty() {
            for &cmds in &[
                &IMEX_COMMANDS[..],
                &DB_COMMANDS[..],
                &CHAT_COMMANDS[..],
                &MESSAGE_COMMANDS[..],
                &CONTACT_COMMANDS[..],
                &MISC_COMMANDS[..],
            ] {
                if let Some(entry) = cmds.iter().find(|el| el.starts_with(&line[..pos])) {
                    if *entry != line && *entry != &line[..pos] {
                        return Some(entry[pos..].to_owned());
                    }
                }
            }
        }
        self.hinter.hint(line, pos, ctx)
    }
}

static COLORED_PROMPT: &str = "\x1b[1;32m> \x1b[0m";
static PROMPT: &str = "> ";

impl Highlighter for DcHelper {
    fn highlight_prompt<'p>(&self, prompt: &'p str) -> Cow<'p, str> {
        if prompt == PROMPT {
            Borrowed(COLORED_PROMPT)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }

    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        self.highlighter.highlight_char(line, pos)
    }
}

impl Helper for DcHelper {}

async fn start(args: Vec<String>) -> Result<(), failure::Error> {
    if args.len() < 2 {
        println!("Error: Bad arguments, expected [db-name].");
        return Err(format_err!("No db-name specified"));
    }
    let context = Context::new("CLI".into(), Path::new(&args[1]).to_path_buf()).await?;

    let ctx = context.clone();
    async_std::task::spawn(async move {
        loop {
            if ctx.has_next_event() {
                if let Ok(event) = ctx.get_next_event() {
                    receive_event(event);
                }
            } else {
                async_std::task::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    });

    println!("Delta Chat Core is awaiting your commands.");

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .output_stream(OutputStreamType::Stdout)
        .build();
    let h = DcHelper {
        completer: FilenameCompleter::new(),
        highlighter: MatchingBracketHighlighter::new(),
        hinter: HistoryHinter {},
    };
    let mut rl = Editor::with_config(config);
    rl.set_helper(Some(h));
    rl.bind_sequence(KeyPress::Meta('N'), Cmd::HistorySearchForward);
    rl.bind_sequence(KeyPress::Meta('P'), Cmd::HistorySearchBackward);
    if rl.load_history(".dc-history.txt").is_err() {
        println!("No previous history.");
    }

    let mut selected_chat = ChatId::default();

    loop {
        let p = "> ";
        let readline = rl.readline(&p);
        match readline {
            Ok(line) => {
                // TODO: ignore "set mail_pw"
                rl.add_history_entry(line.as_str());
                match handle_cmd(line.trim(), context.clone(), &mut selected_chat).await {
                    Ok(ExitResult::Continue) => {}
                    Ok(ExitResult::Exit) => break,
                    Err(err) => println!("Error: {}", err),
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                println!("Exiting...");
                context.stop().await;
                break;
            }
            Err(err) => {
                println!("Error: {}", err);
                break;
            }
        }
    }

    rl.save_history(".dc-history.txt")?;
    println!("history saved");

    Ok(())
}

#[derive(Debug)]
enum ExitResult {
    Continue,
    Exit,
}

async fn handle_cmd(
    line: &str,
    ctx: Context,
    selected_chat: &mut ChatId,
) -> Result<ExitResult, failure::Error> {
    let mut args = line.splitn(2, ' ');
    let arg0 = args.next().unwrap_or_default();
    let arg1 = args.next().unwrap_or_default();

    match arg0 {
        "connect" => {
            ctx.run().await;
        }
        "disconnect" => {
            ctx.stop().await;
        }
        "configure" => {
            ctx.configure().await?;
        }
        "oauth2" => {
            if let Some(addr) = ctx.get_config(config::Config::Addr).await {
                let oauth2_url =
                    dc_get_oauth2_url(&ctx, &addr, "chat.delta:/com.b44t.messenger").await;
                if oauth2_url.is_none() {
                    println!("OAuth2 not available for {}.", &addr);
                } else {
                    println!("Open the following url, set mail_pw to the generated token and server_flags to 2:\n{}", oauth2_url.unwrap());
                }
            } else {
                println!("oauth2: set addr first.");
            }
        }
        "clear" => {
            println!("\n\n\n");
            print!("\x1b[1;1H\x1b[2J");
        }
        "getqr" | "getbadqr" => {
            ctx.run().await;
            if let Some(mut qr) =
                dc_get_securejoin_qr(&ctx, ChatId::new(arg1.parse().unwrap_or_default())).await
            {
                if !qr.is_empty() {
                    if arg0 == "getbadqr" && qr.len() > 40 {
                        qr.replace_range(12..22, "0000000000")
                    }
                    println!("{}", qr);
                    let output = Command::new("qrencode")
                        .args(&["-t", "ansiutf8", qr.as_str(), "-o", "-"])
                        .output()
                        .expect("failed to execute process");
                    io::stdout().write_all(&output.stdout).unwrap();
                    io::stderr().write_all(&output.stderr).unwrap();
                }
            }
        }
        "joinqr" => {
            ctx.run().await;
            if !arg0.is_empty() {
                dc_join_securejoin(&ctx, arg1).await;
            }
        }
        "exit" | "quit" => return Ok(ExitResult::Exit),
        _ => cmdline(ctx.clone(), line, selected_chat).await?,
    }

    Ok(ExitResult::Continue)
}

fn main() -> Result<(), failure::Error> {
    let _ = pretty_env_logger::try_init();

    let args = std::env::args().collect();
    async_std::task::block_on(async move { start(args).await })?;

    Ok(())
}
