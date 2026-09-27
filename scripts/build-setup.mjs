import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, openSync, closeSync, readFileSync, readdirSync, unlinkSync } from 'node:fs';
import { dirname, join, delimiter } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const target = 'x86_64-pc-windows-msvc';
const targetDir = join(root, 'target');
const releaseDir = join(targetDir, target, 'release');
const desktop = join(root, 'apps', 'desktop');
const tauriDir = join(desktop, 'src-tauri');
const staging = join(tauriDir, 'binaries');
const cli = join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
const lockPath = join(targetDir, 'build-setup.lock');
const env = { ...process.env, CARGO_TARGET_DIR: targetDir };
// Tauri's frontend hook must use the same Node installation as this script.
env.PATH = [dirname(process.execPath), join(root, 'node_modules', '.bin'), process.env.PATH ?? ''].join(delimiter);

function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, env, stdio: 'inherit', windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.signal ?? result.status}).`);
}

function build() {
  if (process.argv.length > 2) throw new Error('Usage: npm run build:setup (Windows x64 installer)');
  if (process.platform !== 'win32') throw new Error('Build the Windows setup on Windows with the MSVC toolchain.');
  if (!existsSync(cli)) throw new Error('Frontend dependencies are missing. Run npm ci first.');

  const config = JSON.parse(readFileSync(join(tauriDir, 'tauri.conf.json'), 'utf8'));
  const workspaceVersion = readFileSync(join(root, 'Cargo.toml'), 'utf8')
    .match(/\[workspace\.package\][\s\S]*?\bversion\s*=\s*"([^"]+)"/)?.[1];
  for (const manifest of [join(root, 'package.json'), join(desktop, 'package.json')]) {
    if (JSON.parse(readFileSync(manifest, 'utf8')).version !== config.version) {
      throw new Error(`Version mismatch in ${manifest}; expected ${config.version}.`);
    }
  }
  if (workspaceVersion !== config.version) throw new Error('Cargo and Tauri versions must match.');

  mkdirSync(targetDir, { recursive: true });
  let lock;
  try {
    lock = openSync(lockPath, 'wx');
  } catch (error) {
    if (error.code === 'EEXIST') {
      throw new Error(`Another setup build may be running. If it was interrupted, remove ${lockPath} before retrying.`);
    }
    throw error;
  }
  try {
    console.log(`Building Orion ${config.version} setup (${target})...`);
    run('cargo', ['build', '--locked', '--release', '-p', 'orion-server', '--target', target]);
    mkdirSync(staging, { recursive: true });
    copyFileSync(join(releaseDir, 'orion-server.exe'), join(staging, `orion-server-${target}.exe`));

    run(process.execPath, [cli, 'build', '--target', target, '--bundles', 'nsis',
      '--config', join(tauriDir, 'tauri.setup.conf.json'), '--', '--locked'], desktop);

    const bundleDir = join(releaseDir, 'bundle', 'nsis');
    const installers = readdirSync(bundleDir).filter(name => name.endsWith('-setup.exe') && name.includes(`_${config.version}_`));
    if (installers.length === 0) throw new Error('Tauri completed without producing the expected setup executable.');
    for (const name of installers) console.log(`Setup: ${join(bundleDir, name)}`);
  } finally {
    closeSync(lock);
    unlinkSync(lockPath);
  }
}

try {
  build();
} catch (error) {
  console.error(`Setup build failed: ${error.message}`);
  process.exitCode = 1;
}
