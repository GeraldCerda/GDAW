use soundcraft_dsp::Category;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PluginGrouping {
    #[default]
    Developer,
    Effect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PluginFormat {
    #[default]
    All,
    BuiltIn,
    Clap,
    Vst3,
    Au,
}

impl PluginFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All formats",
            Self::BuiltIn => "Built-in",
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Au => "AU",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrowserPlugin {
    pub id: String,
    pub name: String,
    pub developer: String,
    pub effect: String,
    pub format: PluginFormat,
    pub is_instrument: bool,
}

pub fn effect_group(category: Category, instrument: bool, name: &str, tags: &[String]) -> String {
    if instrument {
        return "Instruments".into();
    }
    let metadata = tags.join(" ").to_lowercase().replace(['-', '_', '|'], " ");
    let name = name.to_lowercase().replace(['-', '_', '|'], " ");
    let classify = |text: &str| {
        if text.split_whitespace().any(|word| matches!(word, "eq" | "equalizer" | "equaliser" | "filter")) {
            Some("EQs & Filters")
        } else if text.contains("deesser") || text.contains("de esser") {
            Some("De-essers")
        } else if text.contains("limiter") {
            Some("Limiters")
        } else if text.contains("compressor") || text.contains("compression") {
            Some("Compressors")
        } else if text.contains("gate") || text.contains("expander") {
            Some("Gates & Expanders")
        } else if text.contains("reverb") {
            Some("Reverbs")
        } else if text.contains("delay") {
            Some("Delays")
        } else {
            None
        }
    };
    classify(&metadata).or_else(|| classify(&name)).unwrap_or(match category {
        Category::Eq => "EQs & Filters",
        Category::Dynamics => "Dynamics",
        Category::PitchShift => "Pitch",
        Category::Reverb => "Reverbs",
        Category::Delay => "Delays",
        Category::Modulation => "Modulation",
        Category::Harmonic => "Distortion & Saturation",
        Category::Dither => "Dither",
        Category::Instrument => "Instruments",
        Category::Other => "Other Effects",
    }).into()
}

fn entry(id: &str, name: &str, developer: &str, category: Category, instrument: bool, format: PluginFormat, tags: &[String]) -> BrowserPlugin {
    BrowserPlugin {
        id: id.into(),
        name: name.into(),
        developer: if developer.trim().is_empty() { "Unknown Developer".into() } else { developer.trim().into() },
        effect: effect_group(category, instrument, name, tags),
        format,
        is_instrument: instrument,
    }
}

pub fn built_in_catalog() -> Vec<BrowserPlugin> {
    soundcraft_dsp::plugins().iter().map(|plugin| {
        entry(plugin.id, plugin.name, "GDAW", plugin.category, plugin.is_instrument, PluginFormat::BuiltIn, &[])
    }).collect()
}

pub fn catalog(rescan: bool) -> Vec<BrowserPlugin> {
    let mut plugins = built_in_catalog();
    for format in [PluginFormat::Au, PluginFormat::Vst3, PluginFormat::Clap] {
        plugins.extend(catalog_format(format, rescan));
    }
    normalize(&mut plugins);
    plugins
}

pub fn catalog_format(format: PluginFormat, rescan: bool) -> Vec<BrowserPlugin> {
    let mut plugins = Vec::new();
    let clap = if format != PluginFormat::Clap { Vec::new() } else if rescan { soundcraft_clap_host::rescan() } else { soundcraft_clap_host::cached().unwrap_or_else(soundcraft_clap_host::scan) };
    for plugin in clap {
        plugins.push(entry(&plugin.id, &plugin.name, &plugin.vendor, plugin.category, plugin.is_instrument, PluginFormat::Clap, &plugin.features));
    }
    let vst3 = if format != PluginFormat::Vst3 { Vec::new() } else if rescan { soundcraft_vst3_host::rescan() } else { soundcraft_vst3_host::cached().unwrap_or_else(soundcraft_vst3_host::scan) };
    for plugin in vst3 {
        plugins.push(entry(&plugin.id, &plugin.name, &plugin.vendor, plugin.category, plugin.is_instrument, PluginFormat::Vst3, &plugin.sub_categories));
    }
    let au = if format != PluginFormat::Au { Vec::new() } else if rescan { soundcraft_au_host::rescan() } else { soundcraft_au_host::scan() };
    for plugin in au {
        let developer = if plugin.vendor.trim().is_empty() { &plugin.manufacturer } else { &plugin.vendor };
        plugins.push(entry(&plugin.id, &plugin.name, developer, plugin.category, plugin.is_instrument, PluginFormat::Au, &[]));
    }
    normalize(&mut plugins);
    plugins
}

pub fn normalize(plugins: &mut Vec<BrowserPlugin>) {
    plugins.sort_by_cached_key(|plugin| plugin.id.clone());
    plugins.dedup_by(|a, b| a.id == b.id);
    reconcile_effect_groups(plugins);
}

fn reconcile_effect_groups(plugins: &mut [BrowserPlugin]) {
    let mut known = BTreeMap::new();
    for plugin in plugins.iter().filter(|plugin| !plugin.is_instrument && plugin.effect != "Other Effects") {
        let key = (plugin.developer.to_lowercase(), plugin.name.to_lowercase());
        let group = known.entry(key).or_insert_with(|| plugin.effect.clone());
        if *group == "Dynamics" && plugin.effect != "Dynamics" {
            *group = plugin.effect.clone();
        }
    }
    for plugin in plugins.iter_mut().filter(|plugin| !plugin.is_instrument && matches!(plugin.effect.as_str(), "Other Effects" | "Dynamics")) {
        let key = (plugin.developer.to_lowercase(), plugin.name.to_lowercase());
        if let Some(group) = known.get(&key) {
            plugin.effect.clone_from(group);
        }
    }
}

pub fn groups(plugins: &[BrowserPlugin], grouping: PluginGrouping, format: PluginFormat, instruments: Option<bool>, query: &str) -> Vec<(String, Vec<BrowserPlugin>)> {
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut groups = BTreeMap::<String, (String, Vec<BrowserPlugin>)>::new();
    for plugin in plugins {
        if (format != PluginFormat::All && plugin.format != format) || instruments.is_some_and(|instrument| plugin.is_instrument != instrument) {
            continue;
        }
        let searchable = format!("{} {} {} {}", plugin.name, plugin.developer, plugin.effect, plugin.format.label()).to_lowercase();
        if !words.iter().all(|word| searchable.contains(word)) {
            continue;
        }
        let group = match grouping {
            PluginGrouping::Developer => &plugin.developer,
            PluginGrouping::Effect => &plugin.effect,
        };
        groups.entry(group.to_lowercase()).or_insert_with(|| (group.clone(), Vec::new())).1.push(plugin.clone());
    }
    groups.into_values().map(|(name, mut plugins)| {
        plugins.sort_by_cached_key(|plugin| (plugin.name.to_lowercase(), plugin.format.label(), plugin.id.clone()));
        (name, plugins)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_groups_use_host_metadata_and_name_fallbacks() {
        assert_eq!(effect_group(Category::Other, false, "Processor", &["Fx".into(), "Compressor".into()]), "Compressors");
        assert_eq!(effect_group(Category::Dynamics, false, "Vocal De-Esser", &[]), "De-essers");
        assert_eq!(effect_group(Category::Other, false, "Channel EQ", &[]), "EQs & Filters");
        assert_eq!(effect_group(Category::Dynamics, false, "Brickwall Limiter", &[]), "Limiters");
        assert_eq!(effect_group(Category::Other, true, "Sampler", &[]), "Instruments");
        assert_eq!(effect_group(Category::Other, false, "Unclassified", &[]), "Other Effects");
    }

    #[test]
    fn equivalent_au_and_vst3_devices_share_reported_effect_groups() {
        let mut plugins = vec![
            entry("vst3:A", "Processor", "Developer", Category::Eq, false, PluginFormat::Vst3, &[]),
            entry("au:A", "Processor", "Developer", Category::Other, false, PluginFormat::Au, &[]),
            entry("au:B", "Processor", "Another", Category::Other, false, PluginFormat::Au, &[]),
        ];
        reconcile_effect_groups(&mut plugins);
        assert_eq!(plugins[1].effect, "EQs & Filters");
        assert_eq!(plugins[2].effect, "Other Effects");
    }

    #[test]
    fn formats_stay_distinct_under_developer_and_effect_groups() {
        let plugins = vec![
            entry("vst3:A", "Compressor", " Developer ", Category::Dynamics, false, PluginFormat::Vst3, &[]),
            entry("au:A", "Compressor", "developer", Category::Dynamics, false, PluginFormat::Au, &[]),
            entry("au:B", "EQ", "Another", Category::Eq, false, PluginFormat::Au, &[]),
            entry("clap:C", "Synth", "Developer", Category::Instrument, true, PluginFormat::Clap, &[]),
        ];
        let developers = groups(&plugins, PluginGrouping::Developer, PluginFormat::All, Some(false), "");
        assert_eq!(developers.len(), 2);
        assert_eq!(developers[1].1.len(), 2);
        let effects = groups(&plugins, PluginGrouping::Effect, PluginFormat::All, Some(false), "");
        assert_eq!(effects[0].0, "Compressors");
        assert_eq!(effects[0].1.len(), 2);
        let au = groups(&plugins, PluginGrouping::Effect, PluginFormat::Au, None, "developer compressor AU");
        assert_eq!(au.len(), 1);
        assert_eq!(au[0].1.len(), 1);
        assert_eq!(au[0].1[0].id, "au:A");
        assert!(groups(&plugins, PluginGrouping::Developer, PluginFormat::All, Some(true), "eq").is_empty());
    }
}
