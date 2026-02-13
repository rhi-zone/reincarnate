/** GML ini file functions — backed by localStorage. */

let iniPath = "";
let iniContents: Record<string, Record<string, string>> = {};
let gameName = "";

export function setGameName(name: string): void {
  gameName = name;
}

export function ini_open(path: string): void {
  iniPath = path;
  const raw = localStorage.getItem("__gml_fs_" + gameName + "_" + path);
  ini_open_from_string(raw);
}

export function ini_open_from_string(str: string | null): void {
  if (!str) {
    iniContents = {};
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
  iniContents = sections;
}

export function ini_read_real(section: string, key: string, defaultVal: number): number {
  return +ini_read_string(section, key, String(defaultVal));
}

export function ini_read_string(section: string, key: string, defaultVal: string): string {
  const val = (iniContents[section] || {})[key];
  return val === undefined ? defaultVal : val;
}

export function ini_write_real(section: string, key: string, value: number): void {
  ini_write_string(section, key, String(value));
}

export function ini_write_string(section: string, key: string, value: string): void {
  if (iniContents[section] === undefined) {
    iniContents[section] = {};
  }
  iniContents[section][key] = String(value);
}

export function ini_section_exists(section: string): boolean {
  return iniContents[section] !== undefined;
}

export function ini_key_exists(section: string, key: string): boolean {
  return iniContents[section] !== undefined && iniContents[section][key] !== undefined;
}

export function ini_section_delete(section: string): void {
  delete iniContents[section];
}

export function ini_key_delete(section: string, key: string): void {
  if (iniContents[section]) {
    delete iniContents[section][key];
  }
}

export function ini_close(): string {
  let result = "";
  for (const section in iniContents) {
    result += `[${section}]\n`;
    for (const key in iniContents[section]) {
      result += `${key}=${iniContents[section][key]}\n`;
    }
    result += "\n";
  }
  localStorage.setItem("__gml_fs_" + gameName + "_" + iniPath, result);
  iniPath = "";
  iniContents = {};
  return result;
}
