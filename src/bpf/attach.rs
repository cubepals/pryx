// SPDX-License-Identifier: FSL-1.1-ALv2
//! One sandbox's programs in the kernel: loaded from the embedded object, its maps written,
//! attached to its cgroup through links, and links and maps pinned so they outlive this daemon.
//! A restarted daemon finds them by their pins and checks each link is still the one it made;
//! detaching removes the pins, which detaches the programs and frees the maps. What the rules
//! say is `rules.rs`; which cgroup and pin directory is `host.rs`.
use super::object::{self, CONFIG_BYTES, Counter, CounterIndex, PROGRAMS, RULE_BYTES, Rule, SOCKET_BYTES};
use aya::{
    EbpfLoader,
    maps::{Array, LpmTrie, Map, MapData, PerCpuArray, SkStorage, lpm_trie::Key},
    programs::{CgroupAttachMode, CgroupSkbAttachType, Program, links::FdLink, links::PinnedLink},
};
use ipnet::Ipv6Net;
use std::{
    collections::BTreeMap,
    fs,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

/// A program the kernel holds for one sandbox: its link's and program's ids.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Attachment {
    pub(crate) program: String,
    pub(crate) link: u32,
    pub(crate) id: u32,
}

pub(crate) struct Programs {
    pins: PathBuf,
    rules: LpmTrie<MapData, [u8; 16], [u8; RULE_BYTES]>,
    config: Array<MapData, [u8; CONFIG_BYTES]>,
    sockets: SkStorage<MapData, [u8; SOCKET_BYTES]>,
    counters: PerCpuArray<MapData, Counter>,
    pub(crate) attachments: Vec<Attachment>,
}

/// Traffic one sandbox's programs counted, summed over every CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) ingress: Counter,
    pub(crate) egress: Counter,
    pub(crate) refused_addresses: u64,
    pub(crate) refused_sockets: u64,
    pub(crate) refused_ingress: Counter,
}

/// What failed and every cause the error carries: the kernel's errno is a source, not the message.
fn error(what: &str, problem: impl std::error::Error) -> String {
    let mut text = format!("{what}: {problem}");
    let mut cause = problem.source();
    while let Some(next) = cause {
        text.push_str(&format!(": {next}"));
        cause = next.source();
    }
    text
}

/// How a verifier's refusal starts, so a reader can tell it from a missing privilege.
pub(crate) const REJECTED: &str = "the verifier rejected";
/// The end of a verifier's log kept with its refusal: the last lines say why.
const LOG_TAIL: usize = 1500;

/// A verifier's refusal, with the end of the log that says why.
fn load_error(name: &str, problem: aya::programs::ProgramError) -> String {
    match problem {
        aya::programs::ProgramError::LoadError { io_error, verifier_log } => {
            let log = verifier_log.to_string();
            let start = log.char_indices().map(|(index, _)| index).find(|index| log.len() - index <= LOG_TAIL);
            format!("{REJECTED} {name}: {io_error}\n{}", &log[start.unwrap_or(log.len())..])
        }
        other => error(&format!("loading {name}"), other),
    }
}

/// A cgroup link takes no attach flags; the kernel runs it beside every other program attached
/// to the cgroup and its ancestors (BPF_F_ALLOW_MULTI semantics, kernel/bpf/cgroup.c).
const LINKED: CgroupAttachMode = CgroupAttachMode::Single;

fn link_pin(pins: &Path, program: &str) -> PathBuf {
    pins.join(format!("link-{program}"))
}

fn map_pin(pins: &Path, map: &str) -> PathBuf {
    pins.join(format!("map-{map}"))
}

fn key(prefix: &Ipv6Net) -> Key<[u8; 16]> {
    Key::new(u32::from(prefix.prefix_len()), prefix.network().octets())
}

/// Every pin under a sandbox's directory, and the directory itself; what is already gone is fine.
pub(crate) fn remove_pins(pins: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(pins) {
        Ok(entries) => entries,
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(problem) => return Err(error("reading pins", problem)),
    };
    // Links first, so no program runs against a half-removed policy.
    let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    paths.sort_by_key(|path| !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("link-")));
    for path in paths {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => {}
            Err(problem) => return Err(error(&format!("removing {}", path.display()), problem)),
        }
    }
    match fs::remove_dir(pins) {
        Ok(()) => Ok(()),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(problem) => Err(error("removing the pin directory", problem)),
    }
}

impl Programs {
    /// Loads every program of `elf` (the embedded object but in a test), writes `rules` and
    /// `config`, attaches each to `cgroup` and pins it under `pins`. Leaves nothing behind when any
    /// step fails, the verifier's refusal of any program included.
    pub(crate) fn attach(
        elf: &[u8],
        pins: &Path,
        cgroup: &Path,
        rules: &BTreeMap<Ipv6Net, Rule>,
        config: [u8; CONFIG_BYTES],
    ) -> Result<Self, String> {
        remove_pins(pins)?;
        fs::create_dir(pins).map_err(|problem| error("making the pin directory", problem))?;
        let attached = Self::attach_pinned(elf, pins, cgroup, rules, config);
        if attached.is_err() {
            let _ = remove_pins(pins);
        }
        attached
    }

    fn attach_pinned(
        elf: &[u8],
        pins: &Path,
        cgroup: &Path,
        rules: &BTreeMap<Ipv6Net, Rule>,
        config: [u8; CONFIG_BYTES],
    ) -> Result<Self, String> {
        let mut ebpf = EbpfLoader::new().load(elf).map_err(|problem| error("loading the object", problem))?;
        let mut take = |name: &str| -> Result<Map, String> {
            let map = ebpf.take_map(name).ok_or_else(|| format!("the object has no {name} map"))?;
            map.pin(map_pin(pins, name)).map_err(|problem| error(&format!("pinning {name}"), problem))?;
            Ok(map)
        };
        let maps = [take(object::RULES)?, take(object::CONFIG)?, take(object::SOCKETS)?, take(object::COUNTERS)?];
        let mut programs = Self::from_maps(pins, maps, Vec::new())?;
        programs.write(rules, config)?;
        let directory = fs::File::open(cgroup).map_err(|problem| error("opening the cgroup", problem))?;
        for name in PROGRAMS {
            let program = ebpf.program_mut(name).ok_or_else(|| format!("the object has no {name} program"))?;
            let link = match program {
                Program::CgroupSockAddr(program) => {
                    program.load().map_err(|problem| load_error(name, problem))?;
                    let id = program.attach(&directory, LINKED);
                    let link = program.take_link(id.map_err(|problem| error(&format!("attaching {name}"), problem))?);
                    FdLink::try_from(link.map_err(|problem| error(name, problem))?)
                }
                Program::CgroupSock(program) => {
                    program.load().map_err(|problem| load_error(name, problem))?;
                    let id = program.attach(&directory, LINKED);
                    let link = program.take_link(id.map_err(|problem| error(&format!("attaching {name}"), problem))?);
                    FdLink::try_from(link.map_err(|problem| error(name, problem))?)
                }
                Program::CgroupSkb(program) => {
                    program.load().map_err(|problem| load_error(name, problem))?;
                    let direction =
                        if name == "ingress" { CgroupSkbAttachType::Ingress } else { CgroupSkbAttachType::Egress };
                    let id = program.attach(&directory, direction, LINKED);
                    let link = program.take_link(id.map_err(|problem| error(&format!("attaching {name}"), problem))?);
                    FdLink::try_from(link.map_err(|problem| error(name, problem))?)
                }
                _ => return Err(format!("{name} is not a cgroup program")),
            }
            .map_err(|problem| error(&format!("{name} has no kernel link"), problem))?;
            let info = link.info().map_err(|problem| error(&format!("reading {name}'s link"), problem))?;
            link.pin(link_pin(pins, name)).map_err(|problem| error(&format!("pinning {name}"), problem))?;
            programs.attachments.push(Attachment { program: name.into(), link: info.id(), id: info.program_id() });
        }
        Ok(programs)
    }

    fn from_maps(pins: &Path, maps: [Map; 4], attachments: Vec<Attachment>) -> Result<Self, String> {
        let [rules, config, sockets, counters] = maps;
        Ok(Self {
            pins: pins.to_path_buf(),
            rules: LpmTrie::try_from(rules).map_err(|problem| error("rules", problem))?,
            config: Array::try_from(config).map_err(|problem| error("config", problem))?,
            sockets: SkStorage::try_from(sockets).map_err(|problem| error("sockets", problem))?,
            counters: PerCpuArray::try_from(counters).map_err(|problem| error("counters", problem))?,
            attachments,
        })
    }

    /// The programs a sandbox had: its pins, each link still the one recorded at attach.
    pub(crate) fn adopt(pins: &Path, recorded: &[Attachment]) -> Result<Self, String> {
        if recorded.len() != PROGRAMS.len() {
            return Err("the journal does not record every program".into());
        }
        for attachment in recorded {
            let link: FdLink = PinnedLink::from_pin(link_pin(pins, &attachment.program))
                .map_err(|problem| error(&format!("{}'s pin", attachment.program), problem))?
                .into();
            let info = link.info().map_err(|problem| error(&format!("{}'s link", attachment.program), problem))?;
            if info.id() != attachment.link || info.program_id() != attachment.id {
                return Err(format!("{}'s link is not the one this daemon attached", attachment.program));
            }
        }
        let open = |name: &str| -> Result<Map, String> {
            let data =
                MapData::from_pin(map_pin(pins, name)).map_err(|problem| error(&format!("{name}'s pin"), problem))?;
            Map::from_map_data(data).map_err(|problem| error(name, problem))
        };
        let maps = [open(object::RULES)?, open(object::CONFIG)?, open(object::SOCKETS)?, open(object::COUNTERS)?];
        Self::from_maps(pins, maps, recorded.to_vec())
    }

    /// Replaces every rule and the gateway's ports. Rules no longer wanted are removed after the
    /// new ones are in, so no lookup misses in between.
    pub(crate) fn write(&mut self, rules: &BTreeMap<Ipv6Net, Rule>, config: [u8; CONFIG_BYTES]) -> Result<(), String> {
        if rules.len() > object::MAX_RULES {
            return Err(format!("the policy needs {} prefixes; at most {} fit", rules.len(), object::MAX_RULES));
        }
        self.config.set(0, &config, 0).map_err(|problem| error("writing the gateway", problem))?;
        for (prefix, rule) in rules {
            let bytes = rule.bytes().ok_or_else(|| format!("{prefix} has too many port ranges"))?;
            self.rules
                .insert(&key(prefix), &bytes, 0)
                .map_err(|problem| error(&format!("writing {prefix}"), problem))?;
        }
        let wanted: std::collections::BTreeSet<(u32, [u8; 16])> =
            rules.keys().map(|prefix| (u32::from(prefix.prefix_len()), prefix.network().octets())).collect();
        let stale: Vec<Key<[u8; 16]>> = self
            .rules
            .keys()
            .filter_map(Result::ok)
            .filter(|key| !wanted.contains(&(key.prefix_len(), key.data())))
            .collect();
        for key in stale {
            self.rules.remove(&key).map_err(|problem| error("removing a stale rule", problem))?;
        }
        Ok(())
    }

    /// Marks a socket pryxd serves this sandbox, so its programs let the sandbox reach it.
    pub(crate) fn admit_gateway(&mut self, socket: &impl AsRawFd) -> Result<(), String> {
        self.sockets.insert(socket, &object::gateway_socket(), 0).map_err(|problem| error("marking a gateway", problem))
    }

    pub(crate) fn counts(&self) -> Result<Counts, String> {
        let read = |index: CounterIndex| -> Result<Counter, String> {
            let values = self.counters.get(&(index as u32), 0).map_err(|problem| error("reading counters", problem))?;
            Ok(values.iter().fold([0, 0], |sum, value| [sum[0] + value[0], sum[1] + value[1]]))
        };
        Ok(Counts {
            ingress: read(CounterIndex::Ingress)?,
            egress: read(CounterIndex::Egress)?,
            refused_addresses: read(CounterIndex::RefusedAddresses)?[0],
            refused_sockets: read(CounterIndex::RefusedSockets)?[0],
            refused_ingress: read(CounterIndex::RefusedIngress)?,
        })
    }

    /// Detaches every program and frees the maps.
    pub(crate) fn detach(self) -> Result<(), String> {
        let pins = self.pins.clone();
        drop(self);
        remove_pins(&pins)
    }
}
