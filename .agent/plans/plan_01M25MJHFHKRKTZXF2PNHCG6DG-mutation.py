import hashlib, json, os, shutil, subprocess, tempfile, tomllib
from pathlib import Path
root = Path('/home/aa/Documents/batter')
subject = Path(tempfile.mkdtemp(prefix='batter-http-graceful-mutation-'))
print('SUBJECT', subject, flush=True)
for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']:
    shutil.copy2(root/name, subject/name)
for name in ['.cargo', 'crates', 'examples', 'test-support', 'scripts']:
    shutil.copytree(root/name, subject/name, ignore=shutil.ignore_patterns('target','__pycache__'))
axum = Path('/home/aa/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/axum-0.8.9')
shutil.copytree(axum, subject/'mutation-axum')
manifest = subject/'Cargo.toml'
manifest.write_text(manifest.read_text()+'\n[patch.crates-io]\naxum = { path = "mutation-axum" }\n')
source = subject/'mutation-axum/src/serve/mod.rs'
original = source.read_text()
old = 'conn.as_mut().graceful_shutdown();'
assert original.count(old)==1
commands=[]

def build(variant, locked):
    cmd=['cargo','test','-p','batter-axum','--test','http_lifetime','--test','http_lifetime_observations','--no-run','--offline','--message-format=json']
    if locked: cmd.append('--locked')
    # Keep the build directory private: relative path patches can collide in a shared cache.
    env=dict(os.environ,CARGO_TARGET_DIR=str(subject/'target'),RUSTUP_TOOLCHAIN='1.98.1')
    p=subprocess.run(cmd,cwd=subject,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180)
    (subject/f'{variant}-build.log').write_text(p.stdout)
    commands.append({'variant':variant,'command':cmd,'status':p.returncode})
    assert p.returncode==0, f'build failure is not mutation evidence: {subject}'
    bins={}
    for line in p.stdout.splitlines():
        try:r=json.loads(line)
        except ValueError:continue
        if r.get('executable'):bins[r['target']['name']]=r['executable']
    assert set(bins)=={'http_lifetime','http_lifetime_observations'},bins
    return bins

cases={
 'http_lifetime':['admitted_handler_completes_during_drain','response_context_completion_does_not_finish_streaming_body'],
 'http_lifetime_observations':['admitted_handler_drain','streaming_cooperative_drain'],
}
runs=[]
for variant in ['original','premature-close','producer-only']:
    if variant=='premature-close':source.write_text(original.replace(old,'break; // Mutation: terminate the live connection immediately.'))
    if variant=='producer-only':
        message='signal received in task, starting graceful shutdown'
        assert original.count(message)==1
        source.write_text(original.replace(message,'connection graceful event deliberately unavailable'))
    bins=build(variant,variant!='original')
    for target,names in cases.items():
        for name in names:
            cmd=[bins[target],'--exact',name,'--nocapture']
            p=subprocess.run(cmd,cwd=subject,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=35)
            (subject/f'{variant}-{name}.log').write_text(p.stdout)
            expected = ('must survive native graceful shutdown before release' if target=='http_lifetime'
                        else 'expected pending wire while handler/body release is withheld')
            assert 'running 1 test' in p.stdout
            if variant=='original':assert p.returncode==0,p.stdout
            else:
                assert p.returncode==101,p.stdout
                if variant=='premature-close':
                    assert expected in p.stdout,p.stdout
                    assert 'missing native connection graceful acknowledgement' not in p.stdout,p.stdout
                    assert 'connection-graceful' in p.stdout,p.stdout
                else:
                    expected='missing native connection graceful acknowledgement'
                    assert expected in p.stdout,p.stdout
                    assert 'graceful-signal-ready' in p.stdout,p.stdout
            runs.append({'variant':variant,'target':target,'test':name,'command':cmd,'status':p.returncode,'expected_assertion':expected if variant!='original' else None})
            print(variant,name,p.returncode,flush=True)
# A local path patch may remove only Axum's registry source/checksum from the copied lock.
a=tomllib.loads((root/'Cargo.lock').read_text());b=tomllib.loads((subject/'Cargo.lock').read_text())
for graph in [a,b]:
    for p in graph['package']:
        if p['name']=='axum':
            p.pop('source',None);p.pop('checksum',None)
assert a==b,'unexpected dependency graph change in isolated mutation'
(subject/'evidence.json').write_text(json.dumps({'subject':str(subject),'commands':commands,'runs':runs,'root_lock_sha256':hashlib.sha256((root/'Cargo.lock').read_bytes()).hexdigest(),'source_sha256':hashlib.sha256(original.encode()).hexdigest(),'mutation':{'old':old,'new':'break;'}},indent=2))
print('Evidence:',subject/'evidence.json',flush=True)
