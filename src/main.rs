use structopt::StructOpt;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use git2::{Commit, Object, Oid, Repository, Time};
use serde_json::{map::Map, value::Value};
use std::collections::HashMap;
use std::{error::Error, path::PathBuf};
use std::str::FromStr;


#[derive(Debug)]
enum OutputFormat {
    Json,
    Csv,
}


impl FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "json" => Ok(OutputFormat::Json),
            "csv" => Ok(OutputFormat::Csv),
            _ => Err("Only JSON and CSV outputs are supported.".into()),
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "example", about = "An example of StructOpt usage.")]
struct Args {
    /// Commit range
    #[structopt()]
    range: String,

    /// Fields to include
    #[structopt()]
    fields: String,

    /// Input file
    #[structopt(long,parse(from_os_str))]
    repo: Option<PathBuf>,

    /// Output format
    #[structopt(short,long,default_value = "json")]
    outputformat: OutputFormat,

    /// Order
    #[structopt(long)]
    oldest_first: bool
}

#[paw::main]
fn main(args: Args) -> Result<(), Box<dyn Error>> {
    // let args: Vec<String> = env::args().collect();
    print_commits(
        &args.range,
        &args.fields,
        args.repo,
        args.outputformat,
        args.oldest_first,
    )?;
    Ok(())
}

fn print_commits(
    revision_range: &str,
    formats_input: &str,
    path: Option<PathBuf>,
    output_format: OutputFormat,
    oldest_first: bool,
) -> Result<(), Box<dyn Error>> {
    let repository: &mut Repository = &mut Repository::open(
        path.unwrap_or_else(|| ".".into())
    )?;

    let formats = formats_input.split(',').collect::<Vec<&str>>();
    let mut reference_map: HashMap<Oid, Vec<String>> = HashMap::new();

    if formats.contains(&"D") {
        for reference_result in repository.references()? {
            let reference = reference_result?;
            let ref_shorthand = match reference.shorthand() {
                Some(shorthand) => shorthand,
                None => continue,
            };
            let ref_target = reference.peel_to_commit()?.id();
            reference_map
                .entry(ref_target)
                .or_insert_with(Vec::new)
                .push(ref_shorthand.to_string());
        }
    }

    let mut revwalk = repository.revwalk()?;
    if oldest_first {
        revwalk.set_sorting(git2::Sort::REVERSE)?;
    }
    revwalk.push_range(revision_range)?;

    let mut prevcommit: Option<Commit> = None;
    // println!("{}", &formats.iter().map(|f| Value::String(f.to_string()).to_string()).collect::<Vec<_>>().join(","));
    let mut print_header = true;
    for oid in revwalk {
        let oidr = oid?;
        let commit = repository.find_commit(oidr)?;

        let prevtree = prevcommit.map(|pc| pc.tree().unwrap());
        let diffstats = repository
            .diff_tree_to_tree(prevtree.as_ref(), commit.tree().ok().as_ref(), None)
            .map(|diff| diff.stats().ok())
            .ok()
            .flatten();

        prevcommit = Some(commit.clone());
        let mut map = Map::new();
        for format in &formats {
            map.insert(format.to_string(), match *format {
                "H" => Value::String(oid_to_hex_string(commit.id())),
                "h" => Value::String(object_to_hex_string(commit.as_object())?),
                "T" => Value::String(oid_to_hex_string(commit.tree_id())),
                "t" => Value::String(object_to_hex_string(commit.tree()?.as_object())?),
                "P" => commit.parent_ids().map(oid_to_hex_string).map(Value::String).collect::<Value>(),
                "p" => commit.parents()
                    .map(|parent| Ok(Value::String(object_to_hex_string(parent.as_object())?)))
                    .collect::<Result<Value, Box<dyn Error>>>()?,
                "an" => Value::String(commit.author().name().ok_or("Author name contains invalid UTF8")?.to_string()),
                "ae" => Value::String(commit.author().email().ok_or("Author email contains invalid UTF8")?.to_string()),
                "aN" | "aE" => invalid_format(format, "Mailmaps not currently supported, consider using `an`/`ae` instead of `aN`/`aE`")?,
                "at" => Value::Number(commit.author().when().seconds().into()),
                "aI" => Value::String(git_time_to_iso8601(commit.author().when())),
                "ad" | "aD" | "ar" | "ai" => invalid_format(format, "Formatted dates not supported, use `aI` and format the date yourself")?,
                "ct" => Value::Number(commit.time().seconds().into()),
                "cI" => Value::String(git_time_to_iso8601(commit.time())),
                "cd" | "cD" | "cr" | "ci" => invalid_format(format, "Formatted dates not supported, use `cI` and format the date yourself")?,
                "d" => invalid_format(format, "Formatted ref names not supported, use `D` and format the names yourself")?,
                "D" => reference_map
                    .remove(&commit.id())
                    .unwrap_or_else(Vec::new)
                    .into_iter()
                    .map(Value::String)
                    .collect::<Value>(),
                "s" => Value::String(commit.summary().ok_or("Commit header contains invalid UTF8")?.to_string()),
                "b" => invalid_format(format, "Body not supported, use `B` and extract the body yourself")?,
                "B" => Value::String(commit.message().ok_or("Commit message contains invalid UTF8")?.to_string()),
                "N" => invalid_format(format, "Notes not currently supported")?,
                "df" => {
                    Value::String(
                        diffstats.as_ref().map(|ds| ds.files_changed().to_string()).unwrap_or_default()
                    )
                },
                "di" => {
                    Value::String(
                        diffstats.as_ref().map(|ds| ds.insertions().to_string()).unwrap_or_default()
                    )
                },
                "dd" => {
                    Value::String(
                        diffstats.as_ref().map(|ds| ds.deletions().to_string()).unwrap_or_default()
                    )
                },
                "GG" | "G?" | "GS" | "GK" => invalid_format(format, "Signatures not currently supported")?,
                _ => invalid_format(format, "Not found")?
            });
        }
        match output_format {
            OutputFormat::Csv => {
                if print_header {
                    let s = map.keys().map(|s| &**s).collect::<Vec<_>>().join(",");
                    println!("{}", s);
                    print_header = false;
                }
                let v = map
                    .values()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                println!("{}", v);
            },
            OutputFormat::Json => {
                println!("{}", Value::Object(map));
            }
        }
    }
    Ok(())
}

fn oid_to_hex_string(oid: Oid) -> String {
    oid.as_bytes()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

fn object_to_hex_string(object: &Object) -> Result<String, Box<dyn Error>> {
    match object.short_id()?.as_str() {
        Some(shorthash) => Ok(shorthash.to_string()),
        None => Err("libgit returned a bad shorthash".into()),
    }
}

fn git_time_to_iso8601(time: Time) -> String {
    let time_without_zone = NaiveDateTime::from_timestamp(time.seconds(), 0);
    let time_with_zone = DateTime::<FixedOffset>::from_utc(
        time_without_zone,
        FixedOffset::east(time.offset_minutes() * 60),
    );
    time_with_zone.to_rfc3339()
}

fn invalid_format(format: &str, reason: &str) -> Result<Value, Box<dyn Error>> {
    Err(format!("Invalid format `{}`: {}", format, reason).into())
}
