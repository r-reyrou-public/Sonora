use crate::store::Store;
use std::path::Path;
use std::sync::Arc;

/// ファイルごとのラベル。データは `Store`（メモリ上）に保持される。
#[derive(Clone)]
pub struct CustomTagService {
    store: Arc<Store>,
}

impl CustomTagService {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    fn key(file_path: &Path) -> String {
        // 大文字小文字・区切り文字(\ と /)の違いを無視して同じファイルとして扱う
        crate::store::path_key(file_path)
    }

    pub fn get_labels_for_file(&self, file_path: &Path) -> Vec<String> {
        let key = Self::key(file_path);
        self.store.read(|lib| lib.labels.get(&key).cloned().unwrap_or_default())
    }

    pub fn set_labels_for_file(&self, file_path: &Path, labels: &[String]) {
        let key = Self::key(file_path);
        let clean: Vec<String> = labels
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        self.store.mutate(|lib| {
            for l in &clean {
                lib.presets.insert(l.clone());
            }
            if clean.is_empty() {
                lib.labels.remove(&key);
            } else {
                lib.labels.insert(key, clean);
            }
        });
    }

    pub fn get_all_preset_labels(&self) -> Vec<String> {
        self.store.read(|lib| {
            let mut all: Vec<String> = lib.presets.iter().cloned().collect();
            // ファイルに割り当てられているラベルも含める
            for labels in lib.labels.values() {
                for l in labels {
                    if !all.contains(l) {
                        all.push(l.clone());
                    }
                }
            }
            all.sort();
            all
        })
    }

    /// 登録済みの全ラベルと、そのラベルを使っている曲数（未使用は 0）
    pub fn get_labels_with_usage(&self) -> Vec<(String, usize)> {
        self.store.read(|lib| {
            let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
            for l in &lib.presets {
                counts.entry(l.clone()).or_insert(0);
            }
            for labels in lib.labels.values() {
                for l in labels {
                    *counts.entry(l.clone()).or_insert(0) += 1;
                }
            }
            counts.into_iter().collect()
        })
    }

    /// ラベル名を変更する（既に同名があれば統合される）
    pub fn rename_label(&self, old: &str, new: &str) {
        let new = new.trim();
        if new.is_empty() || old == new {
            return;
        }
        self.store.mutate(|lib| {
            if lib.presets.remove(old) {
                lib.presets.insert(new.to_string());
            }
            for list in lib.labels.values_mut() {
                for l in list.iter_mut() {
                    if l == old {
                        *l = new.to_string();
                    }
                }
                list.dedup();
                let mut seen = std::collections::HashSet::new();
                list.retain(|l| seen.insert(l.clone()));
            }
        });
    }

    /// ラベルが付いている全ファイルのパス（小文字化済み）
    pub fn all_tagged_paths(&self) -> Vec<String> {
        self.store.read(|lib| lib.labels.keys().cloned().collect())
    }

    /// ファイルの移動に合わせて、ラベルの紐付け先を付け替える
    pub fn relink_path(&self, old: &str, new: &str) {
        let (old_key, new_key) = (Self::key(Path::new(old)), Self::key(Path::new(new)));
        self.store.mutate(|lib| {
            if let Some(labels) = lib.labels.remove(&old_key) {
                let entry = lib.labels.entry(new_key).or_default();
                for l in labels {
                    if !entry.contains(&l) {
                        entry.push(l);
                    }
                }
            }
        });
    }

    /// フォルダにラベルを付け外しし、配下の全ファイルにも同じ変更を反映する（まとめて1回の保存）
    pub fn apply_folder_labels(&self, folder: &Path, files: &[std::path::PathBuf], add: &[String], remove: &[String]) {
        let mut keys: Vec<String> = vec![Self::key(folder)];
        keys.extend(files.iter().map(|f| Self::key(f)));
        self.store.mutate(|lib| {
            for l in add {
                lib.presets.insert(l.clone());
            }
            for key in keys {
                let list = lib.labels.entry(key.clone()).or_default();
                list.retain(|l| !remove.contains(l));
                for l in add {
                    if !list.contains(l) {
                        list.push(l.clone());
                    }
                }
                if list.is_empty() {
                    lib.labels.remove(&key);
                }
            }
        });
    }

    pub fn add_preset_label(&self, label: &str) {
        let clean = label.trim();
        if clean.is_empty() {
            return;
        }
        self.store.mutate(|lib| {
            lib.presets.insert(clean.to_string());
        });
    }

    #[allow(dead_code)]
    pub fn delete_preset_label(&self, label: &str) {
        self.store.mutate(|lib| {
            lib.presets.remove(label);
            for list in lib.labels.values_mut() {
                list.retain(|l| l != label);
            }
            lib.labels.retain(|_, v| !v.is_empty());
        });
    }
}
