use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::config::AiConfig;

// ---------------------------------------------------------------------------
// Platform skills (embedded)
// ---------------------------------------------------------------------------

const PLATFORM_SKILL_MUD_ROOM_GUIDE: &str = r#"---
name: mud-room-guide
description: How to create rooms with exits, descriptions, and properties in the Ruby MUD framework
user-invocable: true
---

# Creating Rooms

Rooms are the basic building blocks of areas. Each room is a Ruby class
that inherits from `Room`.

## File Structure

Place room files in the `rooms/` directory of your area:

```
rooms/
  entrance.rb
  tavern.rb
  market_square.rb
```

## Basic Room

```ruby
class Tavern < Room
  def setup
    set_short "The Rusty Tankard"
    set_long "A dimly lit tavern with rough wooden tables and the smell " \
             "of ale hanging in the air. A barkeeper polishes glasses " \
             "behind a long oak counter."
  end

  def exits
    { "south" => "rooms/market_square" }
  end
end
```

## Key Methods

- `set_short(str)` — One-line room title shown in brief mode
- `set_long(str)` — Full description shown on `look`
- `exits` — Hash mapping direction names to room file paths
- `items` — Hash of examinable details: `{ "counter" => "A sturdy oak counter..." }`

## Exits

Exits connect rooms together. The value is a path relative to the area root:

```ruby
def exits
  {
    "north" => "rooms/entrance",
    "east"  => "rooms/smithy",
    "up"    => "rooms/tower_top"
  }
end
```

## Examinable Items

Add details players can `examine`:

```ruby
def items
  {
    "counter" => "A long oak counter, worn smooth by years of use.",
    "glasses" => "Rows of mismatched drinking glasses line the shelf.",
    "tables"  => "Rough-hewn tables scarred with knife marks and ale stains."
  }
end
```

## Tips

- Room class names must match the filename (CamelCase of snake_case filename)
- Every area needs at least `rooms/entrance.rb` as the default entry point
- Keep descriptions atmospheric but concise (2-4 sentences)
- Use `items` to reward players who explore by examining things
"#;

const PLATFORM_SKILL_MUD_NPC_GUIDE: &str = r#"---
name: mud-npc-guide
description: How to create NPCs with dialogue, behavior, and combat stats
user-invocable: true
---

# Creating NPCs

NPCs (Non-Player Characters) populate rooms and give areas life. Each NPC
is a Ruby class that inherits from `NPC`.

## File Structure

Place NPC files in the `npcs/` directory:

```
npcs/
  barkeeper.rb
  guard.rb
  merchant.rb
```

## Basic NPC

```ruby
class Barkeeper < NPC
  def setup
    set_name "barkeeper"
    set_short "a grizzled barkeeper"
    set_long "A heavyset man with a thick beard and powerful arms. " \
             "He eyes you with professional wariness."
    set_location "rooms/tavern"
  end
end
```

## Dialogue

Add responses to player speech:

```ruby
def dialogue
  {
    "hello"  => "The barkeeper nods. 'What'll it be?'",
    "ale"    => "He slides a foaming tankard across the counter. 'Two coins.'",
    "rumors" => "'They say the old mine is haunted,' he whispers.",
    :default => "The barkeeper shrugs."
  }
end
```

## Combat Stats

For hostile or fightable NPCs:

```ruby
def setup
  set_name "cave_troll"
  set_short "a massive cave troll"
  set_long "A hulking creature with mottled green skin and tusks."
  set_location "rooms/deep_cave"

  set_hp 50
  set_attack 12
  set_defense 8
  set_aggressive true  # attacks players on sight
end
```

## Merchant NPCs

```ruby
def shop_inventory
  {
    "health_potion" => { price: 10, item: "items/health_potion" },
    "iron_sword"    => { price: 25, item: "items/iron_sword" }
  }
end
```

## Tips

- Give NPCs personality through their dialogue and descriptions
- Use `set_location` to place the NPC in a room at area load time
- Dialogue keys are matched case-insensitively against player `say` commands
- The `:default` key catches anything not explicitly matched
"#;

const PLATFORM_SKILL_MUD_ITEM_GUIDE: &str = r#"---
name: mud-item-guide
description: How to create items — weapons, containers, keys, and consumables
user-invocable: true
---

# Creating Items

Items are objects players can pick up, use, equip, or interact with.
Each item is a Ruby class that inherits from `Item`.

## File Structure

Place item files in the `items/` directory:

```
items/
  iron_sword.rb
  health_potion.rb
  rusty_key.rb
  treasure_chest.rb
```

## Basic Item

```ruby
class IronSword < Item
  def setup
    set_name "iron sword"
    set_short "a sturdy iron sword"
    set_long "A well-forged iron sword with a leather-wrapped grip."
    set_weight 3
  end
end
```

## Weapons

```ruby
class IronSword < Item
  def setup
    set_name "iron sword"
    set_short "a sturdy iron sword"
    set_long "A well-forged iron sword with a leather-wrapped grip."
    set_type :weapon
    set_damage 8
    set_weapon_type :sword
    set_weight 3
  end
end
```

## Consumables

```ruby
class HealthPotion < Item
  def setup
    set_name "health potion"
    set_short "a glowing red potion"
    set_long "A small glass vial filled with a luminous crimson liquid."
    set_type :consumable
    set_weight 1
  end

  def on_use(player)
    player.heal(20)
    player.message "You drink the potion and feel warmth spread through you."
    destroy  # remove item after use
  end
end
```

## Containers

```ruby
class TreasureChest < Item
  def setup
    set_name "treasure chest"
    set_short "a heavy wooden chest"
    set_long "An iron-banded wooden chest with a rusted lock."
    set_type :container
    set_locked true
    set_key "items/rusty_key"
  end

  def contents
    ["items/gold_coins", "items/silver_ring"]
  end
end
```

## Keys

```ruby
class RustyKey < Item
  def setup
    set_name "rusty key"
    set_short "a small rusty key"
    set_long "A tarnished iron key with an ornate bow."
    set_type :key
    set_weight 1
  end
end
```

## Tips

- Item class names must match the filename (CamelCase of snake_case filename)
- Use `set_weight` to control inventory limits
- Consumables should call `destroy` after use to remove themselves
- Containers can be locked and require a matching key item to open
"#;

const PLATFORM_SKILL_MUD_TESTING: &str = r#"---
name: mud-testing
description: How to test area code by running game commands to verify changes work
user-invocable: true
---

# Testing Area Code

Use the `play_command` tool to run game commands and verify your area
works correctly. This lets you interact with the MUD as a player would.

## Basic Testing Flow

1. Start in the area entrance room
2. Use `look` to verify room descriptions
3. Navigate with direction commands to test exits
4. Examine objects to verify item descriptions
5. Interact with NPCs to test dialogue

## Essential Commands

```
look              — See the current room description and exits
look at <thing>   — Examine an object or NPC in detail
go <direction>    — Move through an exit (north, south, east, etc.)
inventory         — Check what you're carrying
get <item>        — Pick up an item
drop <item>       — Drop an item
say <message>     — Speak (triggers NPC dialogue)
use <item>        — Use a consumable or interactive item
```

## Example Test Session

After creating a tavern area, verify it step by step:

```
> look
The Rusty Tankard
A dimly lit tavern with rough wooden tables...
Exits: south

> look at counter
A long oak counter, worn smooth by years of use.

> say hello
The barkeeper nods. 'What'll it be?'

> say rumors
'They say the old mine is haunted,' he whispers.

> go south
Market Square
A bustling open square with merchant stalls...
```

## What to Verify

- **Room descriptions** — Do they display correctly? Are they atmospheric?
- **Exits** — Can you move between rooms? Do all connections work both ways?
- **Items** — Can you examine, pick up, and use items?
- **NPCs** — Do they respond to dialogue? Are they in the right rooms?
- **Edge cases** — What happens with invalid commands or directions?

## Debugging Tips

- If a room shows an error, check the class name matches the filename
- If an exit leads nowhere, verify the target path is correct
- If an NPC doesn't respond, check the dialogue keys match
- Use `look` after every action to confirm the game state changed
"#;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
    pub user_invocable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "url")]
pub enum SkillSource {
    Platform,
    GitRepo(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    pub user_invocable: bool,
}

// ---------------------------------------------------------------------------
// SkillsService
// ---------------------------------------------------------------------------

pub struct SkillsService {
    skills: RwLock<Vec<Skill>>,
}

impl SkillsService {
    /// Load platform skills and clone/pull any configured git repos.
    pub async fn new(config: &AiConfig) -> Result<Self> {
        let mut skills = Vec::new();

        // Load embedded platform skills
        for raw in &[
            PLATFORM_SKILL_MUD_ROOM_GUIDE,
            PLATFORM_SKILL_MUD_NPC_GUIDE,
            PLATFORM_SKILL_MUD_ITEM_GUIDE,
            PLATFORM_SKILL_MUD_TESTING,
        ] {
            if let Some(skill) = parse_skill_md(raw, SkillSource::Platform) {
                skills.push(skill);
            }
        }

        // Load skills from configured git repos
        let cache_dir = PathBuf::from(&config.skills_cache_dir);
        for repo_url in &config.skill_repos {
            match load_git_repo_skills(repo_url, &cache_dir).await {
                Ok(repo_skills) => {
                    tracing::info!(
                        repo = %repo_url,
                        count = repo_skills.len(),
                        "loaded skills from git repo"
                    );
                    skills.extend(repo_skills);
                }
                Err(e) => {
                    tracing::warn!(
                        repo = %repo_url,
                        error = %e,
                        "failed to load skills from git repo"
                    );
                }
            }
        }

        tracing::info!(count = skills.len(), "skills service initialized");

        Ok(Self {
            skills: RwLock::new(skills),
        })
    }

    /// List all skills (name, description, source, user_invocable).
    pub async fn list_skills(&self) -> Vec<SkillSummary> {
        let skills = self.skills.read().await;
        skills
            .iter()
            .map(|s| SkillSummary {
                name: s.name.clone(),
                description: s.description.clone(),
                source: s.source.clone(),
                user_invocable: s.user_invocable,
            })
            .collect()
    }

    /// Get a skill by name, returning the full content.
    pub async fn get_skill(&self, name: &str) -> Option<Skill> {
        let skills = self.skills.read().await;
        skills.iter().find(|s| s.name == name).cloned()
    }
}

// ---------------------------------------------------------------------------
// SKILL.md parser (YAML frontmatter)
// ---------------------------------------------------------------------------

/// Parse a SKILL.md string with YAML frontmatter.
fn parse_skill_md(raw: &str, source: SkillSource) -> Option<Skill> {
    let trimmed = raw.trim();

    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---")?;
    let frontmatter = &after_first[..end_idx];
    let body_start = 3 + end_idx + 4;
    let content = trimmed[body_start..].trim().to_string();

    let mut name = String::new();
    let mut description = String::new();
    let mut user_invocable = false;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("user-invocable:") {
            user_invocable = val.trim() == "true";
        }
    }

    if name.is_empty() {
        return None;
    }

    Some(Skill {
        name,
        description,
        content,
        source,
        user_invocable,
    })
}

// ---------------------------------------------------------------------------
// Git repo skill loading
// ---------------------------------------------------------------------------

async fn load_git_repo_skills(repo_url: &str, cache_dir: &Path) -> Result<Vec<Skill>> {
    let repo_name = repo_url
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git");
    let repo_path = cache_dir.join(repo_name);

    let url = repo_url.to_string();
    let path = repo_path.clone();
    tokio::task::spawn_blocking(move || clone_or_update(&url, &path))
        .await
        .context("git task panicked")??;

    let source_url = repo_url.to_string();
    let path = repo_path.clone();
    let skills = tokio::task::spawn_blocking(move || collect_skill_files(&path, &source_url))
        .await
        .context("skill collection task panicked")??;

    Ok(skills)
}

fn clone_or_update(url: &str, path: &Path) -> Result<()> {
    if path.join(".git").exists() || path.join("HEAD").exists() {
        let repo = git2::Repository::open(path)
            .with_context(|| format!("opening cached repo at {}", path.display()))?;

        let mut remote = repo
            .find_remote("origin")
            .context("finding origin remote")?;
        remote.fetch(&["main", "master"], None, None).ok();
        drop(remote);

        let fetch_ref = repo
            .find_reference("refs/remotes/origin/main")
            .or_else(|_| repo.find_reference("refs/remotes/origin/master"))
            .context("finding remote HEAD ref")?;
        let target = fetch_ref.peel_to_commit()?;
        repo.reset(target.as_object(), git2::ResetType::Hard, None)?;
    } else {
        std::fs::create_dir_all(path)
            .with_context(|| format!("creating cache dir {}", path.display()))?;
        git2::Repository::clone(url, path)
            .with_context(|| format!("cloning {} into {}", url, path.display()))?;
    }
    Ok(())
}

fn collect_skill_files(root: &Path, repo_url: &str) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();
    walk_for_skill_files(root, repo_url, &mut skills)?;
    Ok(skills)
}

fn walk_for_skill_files(dir: &Path, repo_url: &str, skills: &mut Vec<Skill>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            walk_for_skill_files(&path, repo_url, skills)?;
        } else if path.file_name().is_some_and(|n| n == "SKILL.md") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            if let Some(skill) =
                parse_skill_md(&content, SkillSource::GitRepo(repo_url.to_string()))
            {
                skills.push(skill);
            }
        }
    }

    Ok(())
}
