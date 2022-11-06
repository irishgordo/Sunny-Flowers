use std::{env, collections::HashSet, num::NonZeroUsize, fmt, fmt::format, str::FromStr};

use serenity::{
    client::Context,
    framework::standard::{
        help_commands,
        macros::{command, help},
        Args, CommandGroup, CommandResult, HelpOptions,
    },
    model::prelude::*,
};

use url::Url;

use crate::{
    checks::*,
    effects::{
        self, display_queue, now_playing,
        queue::{self, EnqueueAt},
    },
    structs::EventConfig,
    utils::SunnyError
};


use tokio_postgres::{NoTls, Error, Config, types::ToSql};
use futures_util::{pin_mut, TryStreamExt};
use tracing::{event, Level};
use sysinfo::{NetworkExt, NetworksExt, ProcessExt, System, SystemExt};

struct TimelineEvent {
    id: i32,
    day: i32,
    month: String,
    event: String, 
    year: String,
    logged_by: String
}

impl fmt::Display for TimelineEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "*id:* {} | *day:* {} | *month:* {} | *event:* {} | *year:* {} | *logged_by:* {}", self.id, self.day, self.month, self.event, self.year, self.logged_by)
    }
}


struct GroupItem {
    id: i32,
    name: String,
    description: String,
    quantity: i32,
    url: String
}

impl fmt::Display for GroupItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "*id:* {} | *name:* {} | *description:* {} | *quantity:* {} | *url:* {}", self.id, self.name, self.description, self.quantity, self.url)
    }
}



#[help]
pub async fn help(
    ctx: &Context,
    msg: &Message,
    args: Args,
    help_options: &'static HelpOptions,
    groups: &[&'static CommandGroup],
    owners: HashSet<UserId>,
) -> CommandResult {
    help_commands::with_embeds(ctx, msg, args, help_options, groups, owners)
        .await
        .ok_or_else(|| SunnyError::log("failed to send"))?;
    Ok(())
}

#[command]
#[only_in(guilds)]
/// Adds Sunny to the user's current voice channel.
pub async fn join(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    // The user's voice channel id
    let voice_channel_id = guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|vs| vs.channel_id)
        .ok_or_else(|| SunnyError::user("Not in a voice"))?;

    let bot_id = ctx.cache.current_user_id().await;
    let same_voice = guild
        .voice_states
        .get(&bot_id)
        .and_then(|vs| vs.channel_id)
        .map_or(false, |id| id == voice_channel_id);

    if same_voice {
        return Err(SunnyError::user("Already in that voice channel!").into());
    }

    let call_m = effects::join(&EventConfig {
        ctx: ctx.clone(),
        guild_id: guild.id,
        text_channel_id: msg.channel_id,
        voice_channel_id,
    })
    .await?;

    effects::deafen(call_m).await;
    msg.channel_id
        .say(&ctx.http, format!("Joined {}", voice_channel_id.mention()))
        .await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
#[checks(In_Voice)]
/// Removes Sunny from the current voice channel and clears the queue.
pub async fn leave(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| SunnyError::log("Couldn't get guild"))?;

    effects::leave(ctx, guild.id).await?;

    msg.reply(&ctx.http, "Left voice").await?;

    Ok(())
}

fn validate_url(mut args: Args) -> Option<String> {
    let mut url: String = args.single().ok()?;

    if url.starts_with('<') && url.ends_with('>') {
        url = url[1..url.len() - 1].to_string();
    }

    Url::parse(&url).ok()?;

    Some(url)
}

#[command]
#[aliases(p)]
#[max_args(1)]
#[only_in(guilds)]
#[usage("<url>")]
#[example("https://www.youtube.com/watch?v=dQw4w9WgXcQ")]
#[checks(In_Voice)]
/// While Sunny is in a voice channel, you may run the play command so that she
/// can start streaming the given video URL.
pub async fn play(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    let url = validate_url(args).ok_or_else(|| SunnyError::user("Unable to parse url"))?;

    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    let len = queue::play(ctx, guild_id, url, EnqueueAt::Back).await?;

    let reply = if len == 1 {
        "Started playing the song".to_string()
    } else {
        format!("Added song to queue: position {}", len - 1)
    };

    msg.reply(&ctx.http, reply).await?;

    Ok(())
}

#[command]
#[aliases(pn)]
#[max_args(1)]
#[only_in(guilds)]
#[usage("<url>")]
#[example("https://www.youtube.com/watch?v=dQw4w9WgXcQ")]
#[checks(In_Voice)]
/// While Sunny is in a voice channel, you may run the play command so that she
/// can start streaming the given video URL.
pub async fn play_next(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    let url = validate_url(args).ok_or_else(|| SunnyError::user("Unable to parse url"))?;

    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    queue::play(ctx, guild_id, url, EnqueueAt::Front).await?;

    msg.reply(&ctx.http, "Added song to front of queue").await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
/// Shuffles your queue badly
pub async fn shuffle(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("Failed to get guild id"))?;

    queue::shuffle(ctx, guild_id).await?;
    msg.reply(&ctx.http, "Queue Shuffled :game_die:!").await?;
    Ok(())
}

#[command]
#[only_in(guilds)]
#[min_args(2)]
#[max_args(2)]
#[usage("<position> <position>")]
#[example("4 2")]
/// Swaps two songs in the queue by their number
pub async fn swap(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("Failed to get guild id"))?;

    let a = args
        .single::<NonZeroUsize>()
        .map_err(|_| SunnyError::user("Invalid arguments"))?;

    let b = args
        .single::<NonZeroUsize>()
        .map_err(|_| SunnyError::user("Invalid arguments"))?;

    let (t1, t2) = queue::swap(ctx, guild_id, a.into(), b.into()).await?;

    msg.reply(
        &ctx.http,
        format!(
            "Swapped `{}` and `{}`",
            effects::get_song(t1.metadata()),
            effects::get_song(t2.metadata())
        ),
    )
    .await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
#[aliases(np)]
/// Shows the currently playing media
pub async fn now_playing(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    now_playing::send_embed(ctx, guild_id, msg.channel_id).await?;

    msg.delete(&ctx.http).await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
#[checks(In_Voice)]
/// Pauses the currently playing
pub async fn pause(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    queue::pause(ctx, guild_id).await?;

    msg.reply(&ctx.http, "Track paused").await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
#[checks(In_Voice)]
/// Resumes the current song if it was paused
pub async fn resume(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    queue::resume(ctx, guild_id).await?;

    msg.reply(&ctx.http, "Track resumed").await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
#[checks(In_Voice)]
/// Skips the currently playing song and starts the next song in the queue.
pub async fn skip(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    let len = queue::skip(ctx, guild_id).await?;

    msg.reply(
        &ctx.http,
        format!(
            "Song skipped: {} in queue.",
            len.checked_sub(1).unwrap_or_default()
        ),
    )
    .await?;
    Ok(())
}

#[command]
#[only_in(guilds)]
#[checks(In_Voice)]
/// Stops playing the current song and clears the current song queue.
pub async fn stop(ctx: &Context, msg: &Message, _args: Args) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    queue::stop(ctx, guild_id).await?;

    msg.reply(&ctx.http, "Queue cleared.").await?;

    Ok(())
}

#[command]
#[only_in(guilds)]
#[aliases(q, queueueueu)]
/// Shows the current queue
pub async fn queue(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    display_queue::send_embed(ctx, guild_id, msg.channel_id).await?;
    Ok(())
}

#[command]
#[only_in(guilds)]
#[aliases(r, remove)]
#[max_args(1)]
#[example("2")]
#[usage("<position>")]
/// Removes a song from the queue by its position
pub async fn remove_at(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let guild_id = msg
        .guild_id
        .ok_or_else(|| SunnyError::log("message guild id could not be found"))?;

    #[allow(clippy::unwrap_used)]
    let index = args
        .single::<NonZeroUsize>()
        .unwrap_or_else(|_| NonZeroUsize::new(1).unwrap());

    let q = queue::remove_at(ctx, guild_id, index).await?;

    msg.reply(
        &ctx.http,
        format!("Removed: `{}`", effects::get_song(q.metadata())),
    )
    .await?;
    Ok(())
}

#[command]
#[only_in(guilds)]
/// Pong
pub async fn ping(ctx: &Context, msg: &Message) -> CommandResult {
    msg.channel_id.say(&ctx.http, "Pong!").await?;
    Ok(())
}

fn get_db_pw() -> String {
    event!(Level::INFO, "Attempting to find database pw..");
    let db_pw = env::var("DB_PW").expect("Environment variable DB_PW not found");
    return db_pw
}

fn slice_iter<'a>(
    s: &'a [&'a (dyn ToSql + Sync)],
) -> impl ExactSizeIterator<Item = &'a dyn ToSql> + 'a {
    s.iter().map(|s| *s as _)
}

#[command]
#[description = "get all the current group items from the group items database"]
#[only_in(guilds)]
/// return group items
pub async fn get_group_items(ctx: &Context, msg: &Message) -> CommandResult {
    // Connect to the database.
    event!(Level::INFO, "Attempting to connect to db...");

    msg.channel_id.say(&ctx.http, ":race_car: ...connecting to database to fetch group_items...").await?;

    let (client, connection) = Config::new()
    .host("localhost")
    .user("sunny")
    .port(5432)
    .password(get_db_pw())
    .dbname("farflungfellowship")
    .connect(NoTls)
    .await
    .unwrap();
    
    // The connection object performs the actual communication with the database,
    // so spawn it off to run on its own.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    event!(Level::INFO, "Connected to db...");
    msg.channel_id.say(&ctx.http, ":thumbsup: ...database connected for fetching group_items...").await?;

    let mut it = client
        .query_raw("SELECT id, name, quantity, description, url FROM group_items", slice_iter(&[]))
        .await?;

    msg.channel_id.say(&ctx.http, "- starting, fetch of group_items... :white_check_mark: -").await?;        
    pin_mut!(it);
    while let Some(row) = it.try_next().await? {
        let item = GroupItem{
            id: row.get("id"),
            name: row.get("name"),
            description: row.get("description"),
            quantity: row.get("quantity"),
            url: row.get("url")
            
        };
        msg.channel_id.say(&ctx.http, "- :sparkles: -").await?;        
        msg.channel_id.say(&ctx.http, item.to_string()).await?;
        msg.channel_id.say(&ctx.http, "- :sparkles: -").await?;        
    }
    msg.channel_id.say(&ctx.http, ":checkered_flag: finished, fetch of group_items...").await?;        

    Ok(())
}

#[command]
#[description = "add an group item to the group item database"]
#[only_in(guilds)]
#[min_args(1)]
#[max_args(4)]
#[usage("<name> | <description> | <quantity> | <url>")]
#[example("bucket | a regular bucket made of wood | 3")]
#[delimiters(" | ")]
/// Swaps two songs in the queue by their number
pub async fn add_group_item(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    msg.channel_id.say(&ctx.http, ":race_car: ..starting add_group_item, must have at least a name for item").await?;        

    let name = args
        .single::<String>()
        .map_err(|_| SunnyError::user("need a name for the item"))?;

    let description = args
        .single::<String>().unwrap_or_default();

    let temp_quantity = args.single::<String>().unwrap_or_default();

    let result_quantity = i32::from_str(&temp_quantity).unwrap_or(1);

    let url = args.single::<String>().unwrap_or_default(); 

    let to_be_added_msg = format!(":fork_and_knife: ...preparing to add: {} - {} - {} - {}", name, description, result_quantity.to_string(), url);

    msg.channel_id.say(&ctx.http, to_be_added_msg).await?;
    
    event!(Level::INFO, "Attempting to connect to db...");

    msg.channel_id.say(&ctx.http, ":alarm_clock: ...connecting to database to fetch add_group_item...").await?;

    let (client, connection) = Config::new()
    .host("localhost")
    .user("sunny")
    .port(5432)
    .password(get_db_pw())
    .dbname("farflungfellowship")
    .connect(NoTls)
    .await
    .unwrap();
    
    // The connection object performs the actual communication with the database,
    // so spawn it off to run on its own.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    event!(Level::INFO, "Connected to db...");
    msg.channel_id.say(&ctx.http, ":thumbsup: ...database connected for adding group_items...").await?;

    let item_name = name.trim();
    let item_description = description.trim();
    let item_url = url.trim();

    let mut _it = client
        .query("insert into public.group_items (name, description, url, quantity) values($1, $2, $3, $4)", &[&item_name, &item_description, &item_url, &result_quantity])
        .await?;

    msg.channel_id.say(&ctx.http, ":thumbsup: added item :toolbox: successfully :star:").await?;


    Ok(())
}


#[command]
#[description = "delete a group item from the database"]
#[only_in(guilds)]
#[min_args(1)]
#[max_args(1)]
#[usage("1")]
#[example("123")]
/// Swaps two songs in the queue by their number
pub async fn delete_group_item(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    msg.channel_id.say(&ctx.http, ":race_car: ..starting delete_group_item, must have the id for the item").await?;        

    let item_id = args
        .single::<String>()
        .map_err(|_| SunnyError::user("need a id for the item"))?;
    
    event!(Level::INFO, "Attempting to connect to db...");
    let db_item_id_sanitize = item_id.trim();
    let db_item = i32::from_str(&db_item_id_sanitize).unwrap_or(1);
    msg.channel_id.say(&ctx.http, ":alarm_clock: ...connecting to database to delete the group item...").await?;

    let (client, connection) = Config::new()
    .host("localhost")
    .user("sunny")
    .port(5432)
    .password(get_db_pw())
    .dbname("farflungfellowship")
    .connect(NoTls)
    .await
    .unwrap();
    
    // The connection object performs the actual communication with the database,
    // so spawn it off to run on its own.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    event!(Level::INFO, "Connected to db...");
    msg.channel_id.say(&ctx.http, ":thumbsup: ...database connected for deleting a group item...").await?;

    let mut _it = client
        .query("delete from public.group_items where id=$1", &[&db_item])
        .await?;

    msg.channel_id.say(&ctx.http, ":thumbsup: deleted item :toolbox: successfully :star:").await?;


    Ok(())
}

#[command]
#[only_in(guilds)]
/// STATS
pub async fn stat_me(ctx: &Context, msg: &Message) -> CommandResult {
    // Please note that we use "new_all" to ensure that all list of
    // components, network interfaces, disks and users are already
    // filled!
    msg.channel_id.say(&ctx.http, ":race_car: - starting stat info! be patient! ...").await?;

    let mut sys = System::new_all();

    // First we update all information of our `System` struct.
    sys.refresh_all();

    // We display all disks' information:
    msg.channel_id.say(&ctx.http, "=> disks:").await?;
    for disk in sys.disks() {
        let diskfound = format!("{:?}", disk);
        msg.channel_id.say(&ctx.http, diskfound).await?;
    }

    // Network interfaces name, data received and data transmitted:
    msg.channel_id.say(&ctx.http, "=> networks:").await?;
    for (interface_name, data) in sys.networks() {
        let intf = format!("{}: {}/{} B", interface_name, data.received(), data.transmitted());
        msg.channel_id.say(&ctx.http, intf).await?;
    }

    // Components temperature:
    msg.channel_id.say(&ctx.http, "=> components:").await?;
    for component in sys.components() {
        let cpt = format!("{:?}", component);
        msg.channel_id.say(&ctx.http, cpt).await?;
    }

    msg.channel_id.say(&ctx.http, "=> system:").await?;
    // RAM and swap information:
    let totalmem = format!("total memory: {} bytes", sys.total_memory());
    let usedmem = format!("used memory : {} bytes", sys.used_memory());
    let totalswap = format!("total swap  : {} bytes", sys.total_swap());
    let usedswap = format!("used swap   : {} bytes", sys.used_swap());

    // Display system information:
    let sysname = format!("System name:             {:?}", sys.name());
    let syskern = format!("System kernel version:   {:?}", sys.kernel_version());
    let sysos = format!("System OS version:       {:?}", sys.os_version());
    let syshostname = format!("System host name:        {:?}", sys.host_name());

    // Number of CPUs:
    let numcpus = format!("NB CPUs: {}", sys.cpus().len());

    // msg discord chunk
    msg.channel_id.say(&ctx.http, totalmem).await?;
    msg.channel_id.say(&ctx.http, usedmem).await?;
    msg.channel_id.say(&ctx.http, totalswap).await?;
    msg.channel_id.say(&ctx.http, usedswap).await?;
    msg.channel_id.say(&ctx.http, sysname).await?;
    msg.channel_id.say(&ctx.http, syskern).await?;
    msg.channel_id.say(&ctx.http, sysos).await?;
    msg.channel_id.say(&ctx.http, syshostname).await?;
    msg.channel_id.say(&ctx.http, numcpus).await?;

    msg.channel_id.say(&ctx.http, ":checkered_flag: FINISHED STAT :sparkles:").await?;
    Ok(())
}


#[command]
#[only_in(guilds)]
/// month info stuff
pub async fn get_month_info(ctx: &Context, msg: &Message) -> CommandResult {
    msg.channel_id.say(&ctx.http, ":sparkles: month info!").await?;
    msg.channel_id.say(&ctx.http, "(1) Hammer - Common-Name: Deepwinter - Holiday: Midwinter").await?;
    msg.channel_id.say(&ctx.http, "(2) Alturiak - Common-Name: The Claw of Winter - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, "(3) Ches - Common-Name: The Claw of the Sunsets - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, "(4) Tarsakh - Common-Name: The Claw of the Storms - Holiday: Greengrass").await?;
    msg.channel_id.say(&ctx.http, "(5) Mirtul - Common-Name: The Melting - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, "(6) Kythorn - Common-Name: The Time of Flowers - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, "(7) Flamerule - Common-Name: Summertide - Holiday: Midsummer").await?;
    msg.channel_id.say(&ctx.http, "(8) Eleasis - Common-Name: Highsun - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, "(9) Eleint - Common-Name: The Fading - Holiday: Highharvestide").await?;
    msg.channel_id.say(&ctx.http, "(10) Marpenoth - Common-Name: Leaffall - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, "(11) Uktar - Common-Name: The Rotting - Holiday: The Feast of the Moon").await?;
    msg.channel_id.say(&ctx.http, "(12) Nightal - Common-Name: The Drawing Down - Holiday: N/A").await?;
    msg.channel_id.say(&ctx.http, ":checkered_flag: month info!").await?;
    Ok(())
}


#[command]
#[description = "get all the current group events from the database"]
#[only_in(guilds)]
/// return group items
pub async fn get_all_group_events(ctx: &Context, msg: &Message) -> CommandResult {
    // Connect to the database.
    event!(Level::INFO, "Attempting to connect to db...");

    msg.channel_id.say(&ctx.http, ":race_car: ...connecting to database to fetch timeline_events...").await?;

    let (client, connection) = Config::new()
    .host("localhost")
    .user("sunny")
    .port(5432)
    .password(get_db_pw())
    .dbname("farflungfellowship")
    .connect(NoTls)
    .await
    .unwrap();
    
    // The connection object performs the actual communication with the database,
    // so spawn it off to run on its own.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    event!(Level::INFO, "Connected to db...");
    msg.channel_id.say(&ctx.http, ":thumbsup: ...database connected for fetching timeline_events...").await?;

    let mut it = client
        .query_raw("SELECT id, day, month, event, year, logged_by FROM timeline_events", slice_iter(&[]))
        .await?;

    msg.channel_id.say(&ctx.http, "- starting, fetch of timeline_events... :white_check_mark: -").await?;        
    pin_mut!(it);
    while let Some(row) = it.try_next().await? {
        let event = TimelineEvent{
            id: row.get("id"),
            day: row.get("day"),
            month: row.get("month"),
            event: row.get("event"),
            year: row.get("year"),
            logged_by: row.get("logged_by")
        };
        msg.channel_id.say(&ctx.http, "- :bookmark_tabs: -").await?;        
        msg.channel_id.say(&ctx.http, event.to_string()).await?;
        msg.channel_id.say(&ctx.http, "- :bookmark_tabs: -").await?;        
    }
    msg.channel_id.say(&ctx.http, ":checkered_flag: finished, fetch of timeline_events...").await?;        

    Ok(())
}

#[command]
#[description = "add a group event"]
#[only_in(guilds)]
#[min_args(3)]
#[max_args(5)]
#[usage("<event> | <logged_by> | <month> | <day> | <year>")]
#[example("it's hammertime cause its hammer time | odo | Hammer | 1 | 1494 DR")]
#[delimiters(" | ")]
/// Swaps two songs in the queue by their number
pub async fn add_group_event(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    msg.channel_id.say(&ctx.http, ":race_car: ..starting add group event, must have at least a blurb about the event & who logged it, as in you & the month cross-ref months, they're propper nouned").await?;        

    let event = args
        .single::<String>()
        .map_err(|_| SunnyError::user("need an event blurb for the event"))?;

    let logged_by = args
        .single::<String>()
        .map_err(|_| SunnyError::user("need who logged this..."))?;

    let month = args
        .single::<String>()
        .map_err(|_| SunnyError::user("need what month this is"))?;

    let day_unchecked = args.single::<String>().unwrap_or_default();

    let result_day = i32::from_str(&day_unchecked).unwrap_or(0);

    let year = args.single::<String>().unwrap_or_default(); 

    let to_be_added_msg = format!(":fork_and_knife: ...preparing to event: {} - {} - {} - {} - {}", event, logged_by, month, result_day.to_string(), year);

    msg.channel_id.say(&ctx.http, to_be_added_msg).await?;
    
    event!(Level::INFO, "Attempting to connect to db...");

    msg.channel_id.say(&ctx.http, ":alarm_clock: ...connecting to database for timeline events...").await?;

    let (client, connection) = Config::new()
    .host("localhost")
    .user("sunny")
    .port(5432)
    .password(get_db_pw())
    .dbname("farflungfellowship")
    .connect(NoTls)
    .await
    .unwrap();
    
    // The connection object performs the actual communication with the database,
    // so spawn it off to run on its own.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    event!(Level::INFO, "Connected to db...");
    msg.channel_id.say(&ctx.http, ":thumbsup: ...database connected for adding timeline events...").await?;

    let db_event = event.trim();
    let db_logged_by = logged_by.trim();
    let db_month = month.trim();
    let db_day = result_day;
    let db_year = year.trim();

    // TODO: wut... this is so gross we have to do this... ugh... its nearly 11PM screw it for now
    // let first_pass_month = month.trim().to_lowercase();
    // let mut second_pass_month: Vec<char> = first_pass_month.chars().collect();
    // second_pass_month[0] = second_pass_month[0].to_uppercase().nth(0).unwrap();
    // let third_pass_month: String = second_pass_month.into_iter().collect();
    // let db_month = &third_pass_month;


    let mut _it = client
        .query("insert into public.timeline_events (event, logged_by, month, day, year) values($1, $2, $3, $4, $5)", &[&db_event, &db_logged_by, &db_month, &db_day, &db_year])
        .await?;

    msg.channel_id.say(&ctx.http, ":thumbsup: added event :toolbox: successfully :star:").await?;


    Ok(())
}