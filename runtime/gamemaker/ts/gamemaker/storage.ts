/** GML ini file functions — backed by localStorage. */

class StorageState {
  iniPath = "";
  iniContents: Record<string, Record<string, string>> = {};
  gameName = "";
}

const storage = new StorageState();

export function setGameName(name: string): void {
  storage.gameName = name;
}

export function ini_open(path: string): void {
  storage.iniPath = path;
  const raw = localStorage.getItem("__gml_fs_" + storage.gameName + "_" + path);
  ini_open_from_string(raw);
}

export function ini_open_from_string(str: string | null): void {
  if (!str) {
    storage.iniContents = {};
    return;
  }
  const sections: Record<string, Record<string, string>> = {};
  const sectionList = str.split(/\s+(?=\[[^\]]+\])/g);
  for (const sectionStr of sectionList) {
    const m = sectionStr.match(/^(?:\[([^\]]+)\])([\s\S]+)/);
    if (!m) continue;
    const [, name, contents] = m;
    const section: Record<string, string> = {};
    const keyList = contents.trim().split(/\s+(?=.+=.+)/g);
    for (const kv of keyList) {
      const km = kv.match(/(.+?)=(.+)/);
      if (km) section[km[1]] = km[2];
    }
    sections[name] = section;
  }
  storage.iniContents = sections;
}

export function ini_read_real(section: string, key: string, defaultVal: number): number {
  return +ini_read_string(section, key, String(defaultVal));
}

export function ini_read_string(section: string, key: string, defaultVal: string): string {
  const val = (storage.iniContents[section] || {})[key];
  return val === undefined ? defaultVal : val;
}

export function ini_write_real(section: string, key: string, value: number): void {
  ini_write_string(section, key, String(value));
}

export function ini_write_string(section: string, key: string, value: string): void {
  if (storage.iniContents[section] === undefined) {
    storage.iniContents[section] = {};
  }
  storage.iniContents[section][key] = String(value);
}

export function ini_section_exists(section: string): boolean {
  return storage.iniContents[section] !== undefined;
}

export function ini_key_exists(section: string, key: string): boolean {
  return storage.iniContents[section] !== undefined && storage.iniContents[section][key] !== undefined;
}

export function ini_section_delete(section: string): void {
  delete storage.iniContents[section];
}

export function ini_key_delete(section: string, key: string): void {
  if (storage.iniContents[section]) {
    delete storage.iniContents[section][key];
  }
}

export function ini_close(): string {
  let result = "";
  for (const section in storage.iniContents) {
    result += `[${section}]\n`;
    for (const key in storage.iniContents[section]) {
      result += `${key}=${storage.iniContents[section][key]}\n`;
    }
    result += "\n";
  }
  localStorage.setItem("__gml_fs_" + storage.gameName + "_" + storage.iniPath, result);
  storage.iniPath = "";
  storage.iniContents = {};
  return result;
}
