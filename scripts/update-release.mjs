// Signed, version-bound update metadata. Private keys are never written to the repository.
import { generateKeyPairSync, createPrivateKey, createPublicKey, sign, verify, createHash } from 'node:crypto';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import assert from 'node:assert/strict';

const [command, ...args] = process.argv.slice(2);
if (command === 'validate') {
  const version = JSON.parse(await readFile('package.json', 'utf8')).version;
  assert.equal((await readFile('src-tauri/Cargo.toml','utf8')).match(/^version = "([^"]+)"/m)?.[1], version);
  assert.equal(JSON.parse(await readFile('src-tauri/tauri.conf.json','utf8')).version, version);
  if (process.env.GITHUB_REF?.startsWith('refs/tags/v')) {
    assert.match(version, /^\d+\.\d+\.\d+$/, 'Stable release tags require a stable version');
    assert.equal(process.env.GITHUB_REF_NAME, `v${version}`);
  }
  assert.equal(Buffer.from((await readFile('src-tauri/update-public-key.txt','utf8')).trim(),'base64').length,32);
} else if (command === 'keygen') {
  const [publicPath, privatePath] = args.map(p => resolve(p));
  if (!publicPath || !privatePath) throw Error('keygen <public-file> <private-file>');
  const keys = generateKeyPairSync('ed25519');
  await mkdir(dirname(privatePath), { recursive: true });
  await writeFile(privatePath, keys.privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'), { flag: 'wx', mode: 0o600 });
  const raw = keys.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  await writeFile(publicPath, raw.toString('base64') + '\n', { flag: 'wx' });
} else if (command === 'manifest') {
  const [binary, output, tag, notesFile] = args;
  const version = JSON.parse(await readFile('package.json', 'utf8')).version;
  assert.match(version, /^\d+\.\d+\.\d+$/, 'Only stable versions enter the update feed');
  assert.equal(tag, `v${version}`, 'Tag and package version must match');
  const cargo = await readFile('src-tauri/Cargo.toml', 'utf8');
  assert.equal(cargo.match(/^version = "([^"]+)"/m)?.[1], version);
  assert.equal(JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8')).version, version);
  const bytes = await readFile(binary);
  const privateKey = createPrivateKey({ key: Buffer.from(process.env.VAULT_UPDATE_SIGNING_KEY || '', 'base64'), format: 'der', type: 'pkcs8' });
  const raw = (await readFile('src-tauri/update-public-key.txt', 'utf8')).trim();
  const expected = createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64');
  assert.equal(expected, raw, 'Signing key does not match the embedded public key');
  const archive = await readFile(resolve(dirname(binary), `../vault-${version}.zip`));
  const payload = Buffer.from(JSON.stringify({ schema: 2, version, platform: 'windows-x86_64', url: `https://github.com/panda-student/Vault/releases/download/${tag}/vault-${version}.zip`, sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length, archive_sha256: createHash('sha256').update(archive).digest('hex'), archive_size: archive.length, notes: notesFile ? (await readFile(notesFile, 'utf8')).slice(0, 16000) : `Vault ${version}` }));
  const signature = sign(null, payload, privateKey);
  assert.ok(verify(null, payload, createPublicKey(privateKey), signature));
  await writeFile(output, JSON.stringify({ payload: payload.toString('base64'), signature: signature.toString('base64') }, null, 2) + '\n');
} else {
  throw Error('Expected keygen or manifest');
}
