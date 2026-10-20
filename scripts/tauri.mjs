import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const cargoBin = join(homedir(), '.cargo', 'bin');
const env = { ...process.env };
// Windows treats Path/PATH as the same variable, while an object copy does not.
// Keep a single spelling so Node does not discard the augmented search path.
const pathKey = Object.keys(env).find(key => key.toLowerCase() === 'path') ?? 'PATH';
env[pathKey] = [dirname(process.execPath), ...(existsSync(cargoBin) ? [cargoBin] : []), env[pathKey] ?? ''].join(delimiter);
const child = spawn(process.execPath, [resolve(root, 'node_modules/@tauri-apps/cli/tauri.js'), ...process.argv.slice(2)], {
  cwd: root, env, stdio: 'inherit', windowsHide: true,
});
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', code => { process.exitCode = code ?? 1; });
