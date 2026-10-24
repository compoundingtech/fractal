#!/usr/bin/env python3
"""Integrate open Fractal PRs and publish an immutable production build.

Version 7: preserve main's derived changelog registry after every merge; retain PR entry files.
"""
import datetime, fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, time
def required_input(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f'missing required input: {name}')
    return value

S = pathlib.Path(required_input('FRACTAL_INTEGRATOR_STATE_ROOT'))
L = S/'live'
C = pathlib.Path(required_input('FRACTAL_INTEGRATOR_CLONE'))
P = pathlib.Path(required_input('FRACTAL_INTEGRATOR_PROD_ROOT'))
purpose = required_input('FRACTAL_INTEGRATOR_PURPOSE')
SOURCE_CLONE = pathlib.Path(required_input('FRACTAL_INTEGRATOR_SOURCE_CLONE'))
PRIVATE_OVERLAY = pathlib.Path(required_input('FRACTAL_INTEGRATOR_PRIVATE_OVERLAY'))
VENDOR = pathlib.Path(required_input('FRACTAL_INTEGRATOR_VENDOR'))
BUILD_SCRIPT = pathlib.Path(required_input('FRACTAL_INTEGRATOR_BUILD_SCRIPT'))
# Required overlays and their ordering belong to the deployment.
LOCAL_HEAD_OVERRIDES = {}
try:
    PRIORITY = json.loads(required_input('FRACTAL_INTEGRATOR_PRIORITY_JSON'))
    if not isinstance(PRIORITY, list) or any(type(n) is not int or n <= 0 for n in PRIORITY):
        raise ValueError('expected an array of positive PR numbers')
except (ValueError, TypeError) as error:
    raise SystemExit(f'invalid FRACTAL_INTEGRATOR_PRIORITY_JSON: {error}')
REQUIRED = frozenset(PRIORITY)
INTEGRATOR_VERSION = 7
DRY_RUN = '--dry-run' in __import__('sys').argv
if DRY_RUN:
    C = pathlib.Path(os.environ['FRACTAL_DRY_RUN_CLONE'])
TIMINGS = None
REGISTRY_PATH = 'apps/fractal-web/.changelog/mod.ts'
REGISTRY_BASE = None

def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + f'.{os.getpid()}.tmp')
    temporary.write_text(json.dumps(value))
    temporary.replace(path)

def candidate_key(private_hash):
    # Tree identity excludes ephemeral merge/stamp commit timestamps.
    return hashlib.sha256((text('rev-parse', 'HEAD^{tree}') + private_hash).encode()).hexdigest()

def publish_skips(row):
    if not DRY_RUN:
        atomic_json(S/'steward/last-skips.json', {
            'ts': row['ts'], 'main': row['main'], 'included': row['included'],
            'skipped': row['skipped'], 'selection': row['selection'],
            'wireContractAllowed': row.get('wireContractAllowed', []),
            'complete': row.get('selectionComplete', False), 'error': row.get('error')})

def first_break(checkpoints, probe):
    """Find the first failing prefix; confirm its immediate predecessor is good."""
    observations={}
    raw_probe=probe
    def probe(revision):
        if revision not in observations:
            observations[revision]=raw_probe(revision)
        return observations[revision]
    if not checkpoints or not probe(checkpoints[0]):
        return None  # main/required stack broken, not attributable to a PR
    low, high = 0, len(checkpoints)-1
    while high-low > 1:
        middle = (low+high)//2
        if probe(checkpoints[middle]): low = middle
        else: high = middle
    if not probe(checkpoints[high-1]) or probe(checkpoints[high]):
        return None
    return high

def swap_server(cleanenv):
    # After the SYSTEM roll, replace this down with up --replace in activate.
    subprocess.run(['devnet','down',purpose],cwd=C/'apps/fractal-web',env=cleanenv,check=True)

def generated(path):
    return pathlib.PurePosixPath(path).name in ('pnpm-lock.yaml', 'licenses.json', 'pnpm-lock.sha256.json')

# This loop deploys only the web app, never the daemon it connects to.
# A changed generated contract requires a coordinated deployment elsewhere.
WIRE_CONTRACT_PATHS = frozenset((
    'clients/typescript/st3-client/Schema.generated.ts',
    'clients/typescript/st3-client/Models.generated.ts',
    'clients/typescript/st3-client/Client.generated.ts',
    'clients/swift/St3Client/Sources/St3Client/Contract.generated.swift',
    'crates/st3-client/src/generated.rs',
    'crates/st3-client/src/contract.rs',
    'docs/st3/client-v0/schemas/client-v0.schema.json',
    'docs/st3/client-v0/schemas/operations.json',
))

def select_pr(pr, row, manual):
    files = [f['path'] for f in pr['files']]
    # Inspect every page, so path selection is independent of GitHub's file cap.
    if len(files) >= 100:
        pages = json.loads(run(['gh', 'api', '--paginate', '--slurp',
            f'repos/compoundingtech/fractal/pulls/{pr["number"]}/files?per_page=100'],
            capture=True, timeout=120).stdout)
        files = [f['filename'] for page in pages for f in page]
        pr['files'] = [{'path': path} for path in files]
    relevant = [path for path in files if path.startswith(('apps/fractal-web/', 'packages/fractal-ui/'))]
    reason = ('manual-drop' if pr['number'] in manual else
        'fractal-paths' if relevant else 'no-fractal-paths')
    row['selection'].append({'pr': pr['number'], 'reason': reason,
        'draft': pr['isDraft'], 'files': relevant})
    if relevant and pr['number'] in manual:
        row['skipped'].append({'pr': pr['number'], 'reason': reason, 'files': []})
    return bool(relevant) and reason == 'fractal-paths'

def allow_wire_contract(pr, row):
    head = f'origin/live-r3-pr-{pr["number"]}'
    paths = sorted(WIRE_CONTRACT_PATHS)
    # GitHub's files API can retain already-main changes after base updates.
    # Avoid a virtual merge for heads with no contract delta at all.
    if not text('diff', '--name-only', f'origin/main...{head}', '--', *paths):
        return True
    merged = git('merge-tree', '--write-tree', 'origin/main', head, capture=True, check=False)
    if merged.returncode not in (0, 1):
        raise RuntimeError(f'cannot inspect wire contract for PR #{pr["number"]}: {merged.stderr}')
    tree = merged.stdout.splitlines()[0]
    files = text('diff', '--name-only', 'origin/main', tree, '--', *paths).splitlines()
    if not files:
        return True
    if pr['number'] in REQUIRED:
        row.setdefault('wireContractAllowed', []).append({'pr': pr['number'], 'files': files,
            'reason': 'required, ran live before the #1473 incident'})
        return True
    row['skipped'].append({'pr': pr['number'], 'reason': 'wire-contract-change', 'files': files})
    for selection in row['selection']:
        if selection['pr'] == pr['number']:
            selection['reason'] = 'wire-contract-change'
    return False

def prepare_clone():
    """Only this disposable integration clone is reset; live serves immutable builds."""
    if git('rev-parse', '--verify', 'MERGE_HEAD', capture=True, check=False).returncode == 0:
        git('merge', '--abort')
    git('checkout', '-f', '-B', 'live-r3', 'origin/main')
    git('clean', '-fd')  # keep ignored dependencies/private overlay, never -x
    cache=L/('live-r3-dry-run-rerere' if DRY_RUN else 'live-r3-rerere')
    cache.mkdir(exist_ok=True)
    rr=pathlib.Path(text('rev-parse', '--absolute-git-dir'))/'rr-cache'
    if rr.exists() and not rr.is_symlink():
        shutil.copytree(rr,cache,dirs_exist_ok=True); shutil.rmtree(rr)
    if not rr.is_symlink(): rr.symlink_to(cache)
    git('config', 'rerere.enabled', 'true')
    git('config', 'rerere.autoupdate', 'true')
    # Detached rerere gc held MERGE_RR.lock during the next commit.
    git('config', 'maintenance.autoDetach', 'false')
    git('config', 'gc.autoDetach', 'false')

def retain_main_registry():
    """Legacy PR registries cannot replace main's directory-derived implementation."""
    git('restore', '--source', REGISTRY_BASE, '--staged', '--worktree', '--', REGISTRY_PATH)
    if git('diff', '--cached', '--quiet', '--', REGISTRY_PATH, capture=True, check=False).returncode:
        git('commit', '-m', 'live-r3: retain main-derived changelog registry')

def resolve_additive():
    """Resolve only additive documentation/registries and regenerated metadata."""
    resolved = []
    for path in text('diff', '--name-only', '--diff-filter=U').splitlines():
        if path == REGISTRY_PATH:
            git('restore', '--source', REGISTRY_BASE, '--staged', '--worktree', '--', path)
            resolved.append(path)
            continue
        if not (path.endswith('.md') or
                pathlib.PurePosixPath(path).name == 'package.json' or generated(path)):
            continue  # Source conflicts cannot be auto-resolved; never read their stages.
        stages = {n: git('show', f':{n}:{path}', capture=True, check=False) for n in (1, 2, 3)}
        if any(stages[n].returncode for n in (2, 3)):
            continue  # delete/modify needs a real decision
        target = C/path
        if path.endswith('.md'):
            import tempfile
            with tempfile.TemporaryDirectory(dir=L) as tmp:
                files = []
                for n in (2, 1, 3):
                    f = pathlib.Path(tmp)/str(n); f.write_text(stages[n].stdout if not stages[n].returncode else ''); files.append(f)
                merged = run(['git', 'merge-file', '--union', '-p', *files], capture=True)
                target.write_text(merged.stdout)
        elif pathlib.PurePosixPath(path).name == 'package.json':
            # Higher-priority integrated values win scalar clashes; retain new keys
            # from both branches (including dependency/script/workspace additions).
            def combine(ours, theirs):
                if isinstance(ours, dict) and isinstance(theirs, dict):
                    return {k: combine(ours[k], theirs[k]) if k in ours and k in theirs
                            else ours[k] if k in ours else theirs[k] for k in dict.fromkeys([*ours, *theirs])}
                if isinstance(ours, list) and isinstance(theirs, list):
                    return ours + [v for v in theirs if v not in ours]
                return ours
            target.write_text(json.dumps(combine(json.loads(stages[2].stdout), json.loads(stages[3].stdout)), indent=2)+'\n')
        elif generated(path):
            target.write_text(stages[2].stdout)
        else:
            continue
        git('add', '--', path); resolved.append(path)
    return resolved

def prepare_dependencies():
    # Materialize the candidate's locked members before regeneration or cache reuse.
    member_inputs=hashlib.sha256()
    for path in ('megarepo.kdl', 'megarepo.lock'):
        member_inputs.update(path.encode()); member_inputs.update((C/path).read_bytes())
    member_key=member_inputs.hexdigest()
    member_state=L/f'{C.name}-members-key.json'
    members=json.loads((C/'megarepo.lock').read_text())['members']
    if (not member_state.exists() or
            json.loads(member_state.read_text())['key'] != member_key or
            any(not (C/'repos'/name).is_dir() for name in members)):
        effect_rev=json.loads((C/'flake.lock').read_text())['nodes']['effect-utils']['locked']['rev']
        run(['nice', '-n', '19', 'nix', 'run',
             f'github:overengineeringstudio/effect-utils/{effect_rev}#megarepo', '--',
             'apply', '--worktree-mode', 'commit', '--git-protocol', 'https'], cwd=C, timeout=1800)
        atomic_json(member_state, {'key': member_key})
    # Cache only deterministic generated metadata, never source resolutions.
    paths=[p for p in text('ls-files').splitlines()
           if p.endswith(('.genie.ts', 'package.json', 'pnpm-lock.yaml', 'pnpm-workspace.yaml', 'pnpm-source-inputs.json', 'flake.lock', 'megarepo.kdl', 'megarepo.lock'))
           or p.startswith(('genie/', 'scripts/ci-fractal-web-licenses', 'buck2/dependencies/'))]
    inputs=hashlib.sha256()
    for path in paths:
        inputs.update(path.encode()); inputs.update((C/path).read_bytes())
    cache=L/'metadata-cache'/inputs.hexdigest()
    if cache.exists():
        if cache.stat().st_size:
            git('apply','--index',cache)
            git('commit','-m','live-r3: reuse generated dependency metadata')
        install()
        return
    before=text('rev-parse','HEAD')
    regenerate()
    cache.parent.mkdir(exist_ok=True)
    cache.write_text(git('diff','--binary',before,'HEAD',capture=True).stdout if before!=text('rev-parse','HEAD') else '')

def probe_revision(revision):
    git('checkout','--detach',revision)
    prepare_dependencies()
    return build('live-r3-probe').returncode == 0

def build_candidate(row, private_hash, checkpoints):
    cache=L/'live-r3-tree-cache.json'
    known=json.loads(cache.read_text()) if cache.exists() else {}
    key=candidate_key(private_hash)
    row['treeHash']=key
    if key in known:
        entry=known[key]
        if entry.get('ejected') is not None:
            row['included']=[h for h in row['included'] if h['pr']!=entry['ejected']]
            row['skipped'].append({'pr':entry['ejected'],'reason':'build-break','files':[]})
        row.update(sha=entry['sha'], build='skipped', reason='known-'+entry['outcome']+'-tree')
        return False
    prepare_dependencies()
    stamp(private_hash)
    row['sha']=text('rev-parse','HEAD')
    if build('live-r3').returncode == 0:
        known[key]={'outcome':'built','sha':row['sha']}
        atomic_json(cache,known)
        return True
    known[key]={'outcome':'failed','sha':row['sha']}
    atomic_json(cache,known)
    full=text('rev-parse','HEAD')
    failed=first_break([c['sha'] for c in checkpoints],probe_revision)
    git('checkout','-B','live-r3',full)
    if failed is None:
        raise RuntimeError('integrated build failed; no attributable single PR; previous build retained')
    culprit=checkpoints[failed]['pr']
    row['skipped'].append({'pr':culprit,'reason':'build-break','files':[]})
    known[key]['ejected']=culprit
    atomic_json(cache,known)
    # Reintegrate every later accepted head, rather than silently publishing a prefix.
    git('checkout','-B','live-r3',checkpoints[failed-1]['sha'])
    row['included']=[h for h in row['included'] if h['pr']!=culprit]
    for checkpoint in checkpoints[failed+1:]:
        result=git('merge','--no-ff','--no-edit',checkpoint['head'],check=False)
        if result.returncode:
            resolved=resolve_additive()
            conflicts=text('diff','--name-only','--diff-filter=U').splitlines()
            if conflicts:
                git('merge','--abort')
                row['included']=[h for h in row['included'] if h['pr']!=checkpoint['pr']]
                row['skipped'].append({'pr':checkpoint['pr'],'reason':'merge-conflict','files':conflicts,'autoResolved':resolved})
            else: git('commit','--no-edit')
        retain_main_registry()
    retry_key=candidate_key(private_hash)
    row['retryTreeHash']=retry_key
    if known.get(retry_key,{}).get('outcome')=='failed':
        raise RuntimeError('ejected candidate is a known failed tree; previous build retained')
    prepare_dependencies()
    stamp(private_hash)
    row['sha']=text('rev-parse','HEAD')
    ok=build('live-r3').returncode == 0
    known[retry_key]={'outcome':'built' if ok else 'failed','sha':row['sha']}
    atomic_json(cache,known)
    if not ok:
        raise RuntimeError('production build failed after one PR ejection; previous build retained')
    return True
def regenerate():
    # Repo-owned generators: manifests first, pnpm lock next, then Buck archive
    # hashes and the license inventory. Once per candidate, never per PR merge.
    # prepare_dependencies materializes locked members; the genie shell no longer does.
    run(['nice', '-n', '19', 'nix', 'develop', '.#genie', '-c', 'genie', '--defer-validation'], cwd=C, timeout=1800)
    run(['nice', '-n', '19', 'pnpm', 'install', '--lockfile-only', '--no-frozen-lockfile'], cwd=C, timeout=900)
    run(['nice', '-n', '19', 'nix', 'develop', '.#genie', '-c', 'genie'], cwd=C, timeout=1800)
    install()  # license generator requires the frozen installed manifests
    run(['nice', '-n', '19', 'python3', 'scripts/ci-fractal-web-licenses'], cwd=C, timeout=900)
    git('add', '-u')
    if text('diff', '--cached', '--name-only'):
        git('commit', '-m', 'live-r3: regenerate merged dependency metadata')

def run(args, cwd=None, capture=False, check=True, timeout=None):
    started=time.monotonic()
    category = ('fetch' if 'fetch' in args else 'merge' if args[0]=='git' else
                'install' if 'pnpm' in args else 'regenerate' if 'genie' in args or 'scripts/ci-fractal-web-licenses' in args else
                'build' if args[0]=='bash' or 'node' in args else 'selection')
    try:
        log_capture=not capture and ('genie' in args or ('pnpm' in args and '--frozen-lockfile' in args))
        result=subprocess.run([str(a) for a in args], cwd=cwd, text=True,
                              stdout=subprocess.PIPE if capture or log_capture else None,
                              stderr=subprocess.PIPE if capture or log_capture else None,
                              check=check and not log_capture, timeout=timeout,
                              env={**os.environ, 'CI':'1'})
        if log_capture:
            if result.stdout: print(result.stdout, end='', flush=True)
            if result.stderr: print(result.stderr, end='', file=__import__('sys').stderr, flush=True)
            if check and result.returncode:
                result.stdout=(result.stdout or '')[-4000:]
                result.stderr=(result.stderr or '')[-4000:]
                result.check_returncode()
        return result
    finally:
        if TIMINGS is not None:
            TIMINGS[category]=TIMINGS.get(category,0)+round((time.monotonic()-started)*1000)
def git(*args, capture=False, check=True): return run(['git','-C',C,*args], capture=capture, check=check)
def text(*args): return git(*args,capture=True).stdout.strip()
def digest(root):
    h=hashlib.sha256()
    for f in sorted(root.rglob('*')):
        if f.is_file(): h.update(str(f.relative_to(root)).encode()); h.update(f.read_bytes())
    return h.hexdigest()
def stamp(private_hash):
    git('commit','--allow-empty','-m','live-r3: production integration','-m',f'Public client/private server; private overlay SHA256: {private_hash}')
def install():
    inputs=hashlib.sha256()
    for path in ('pnpm-lock.yaml', 'pnpm-workspace.yaml', 'megarepo.kdl',
                 'megarepo.lock', 'pnpm-source-inputs.json'):
        inputs.update(path.encode()); inputs.update((C/path).read_bytes())
    lockhash=inputs.hexdigest()
    state=L/f'{C.name}-lockhash'
    if not (C/'node_modules').exists() or not state.exists() or state.read_text()!=lockhash:
        run(['nice','-n','19','nix','develop','.#web','--option','builders','','-c','pnpm','install','--prefer-offline','--frozen-lockfile'],cwd=C,timeout=900); state.write_text(lockhash)
def build(name):
    return run(['bash',BUILD_SCRIPT,C,text('rev-parse','HEAD'),name],check=False)
def current_server_url(cleanenv):
    """A healthy serving generation is not necessarily the current symlink.

    A completed build swaps current before devnet publication; failed publication
    can leave the edge serving the previous immutable build. Identity mismatch
    means stale content, not a dead server, and must not trigger legacy plain up.
    """
    try:
        status=subprocess.run(['devnet','status',purpose],cwd=C/'apps/fractal-web',env=cleanenv,text=True,capture_output=True,timeout=30)
        if status.returncode: return None
        row=json.loads(status.stdout)
        if row.get('status')!='running': return None
        url=row['tags']['run.url.edge']
        response=subprocess.run(['curl','-fsSL','--max-time','15','-w','\n%{http_code}',url],env=cleanenv,text=True,capture_output=True,timeout=20)
        if response.returncode: return None
        html,_,code=response.stdout.rpartition('\n')
        if code!='200': return None
        return url
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired):
        pass
    return None
def activate(previous):
    current=os.path.realpath(P/'current')
    cleanenv={k:v for k,v in os.environ.items() if k not in ('PTY_SESSION','ST_AGENT')}
    cleanenv['PWD']=str(C/'apps/fractal-web')
    # Avoid Devnet's healthy-generation renewal bug; do not interrupt integration.
    if previous==current:
        healthy_url=current_server_url(cleanenv)
        if healthy_url:
            print('current server healthy; skipping devnet up; continuing tick',flush=True)
            return healthy_url
    # Missing/unhealthy current uses plain up, never --replace.
    # Only a changed build needs the existing down/swap/rollback transaction.
    if previous!=current and previous:
        swap_server(cleanenv)
    result=subprocess.run(['devnet','up',purpose,'--ready-path','/','--',str(P/'current/launch.sh'),purpose,str(P/'current')],cwd=C/'apps/fractal-web',env=cleanenv)
    if result.returncode:
        if previous==current:
            raise RuntimeError('devnet current-server ensure failed; current build retained')
        subprocess.run(['devnet','down',purpose],cwd=C/'apps/fractal-web',env=cleanenv,check=False)
        if previous:
            temp=P/f'.rollback-{os.getpid()}'; temp.symlink_to(previous); temp.replace(P/'current')
            (P/'current-sha').write_text(json.loads((P/'current/build-receipt.json').read_text())['buildIdentity']['rev'])
            subprocess.run(['devnet','up',purpose,'--ready-path','/','--',str(P/'current/launch.sh'),purpose,str(P/'current')],cwd=C/'apps/fractal-web',env=cleanenv,check=True)
        raise RuntimeError('devnet activation failed; previous build rolled back if available')
    subprocess.run(['devnet','url',purpose,'--remote','--wait'],cwd=C/'apps/fractal-web',env=cleanenv,check=True,timeout=120)
    status=subprocess.run(['devnet','status',purpose],cwd=C/'apps/fractal-web',env=cleanenv,text=True,capture_output=True,check=True)
    return json.loads(status.stdout)['tags']['run.url.edge']
def observe_served(row):
    """Compare the candidate with the published edge; retain local diagnostics."""
    cleanenv={k:v for k,v in os.environ.items() if k not in ('PTY_SESSION','ST_AGENT')}
    cleanenv['PWD']=str(C/'apps/fractal-web')
    for location, sha_field, error_field in (
            ('--remote','servedSha','servedError'),
            ('--local','localServedSha','localServedError')):
        row[sha_field]=None
        try:
            address=subprocess.run(['devnet','url',purpose,location],cwd=C/'apps/fractal-web',
                                   env=cleanenv,text=True,capture_output=True,timeout=30,check=True)
            url=address.stdout.strip()
            if not url.startswith(('http://','https://')):
                raise ValueError(f'devnet returned no HTTP {location} URL')
            response=subprocess.run(['curl','-fsS','--max-time','15','--',url.rstrip('/')+'/build-identity.json'],
                                    env=cleanenv,text=True,capture_output=True,timeout=20,check=True)
            revision=json.loads(response.stdout)['rev']
            if not isinstance(revision,str) or not re.fullmatch(r'[0-9a-f]{40}',revision):
                raise ValueError('served build identity has no full revision')
            row[sha_field]=revision
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
            row[error_field]=str(error)[:240]
    row['current']=('error' not in row and row.get('sha') is not None
                    and row['sha']==row['servedSha'])

def tick():
    global TIMINGS, REGISTRY_BASE
    start=time.monotonic(); row={'ts':datetime.datetime.now(datetime.timezone.utc).isoformat(),'sha':None,'main':None,'included':[],'skipped':[],'selection':[],'build':'failed','ms':0}
    row['integratorVersion']=INTEGRATOR_VERSION
    TIMINGS=row['timingsMs']={}
    pin_path=L/'live-r3-pin.json'
    pin=json.loads(pin_path.read_text()) if pin_path.exists() else None
    row['pinnedBase']=pin['main'] if pin else None
    lock=open(L/('live-r3-dry-run.lock' if DRY_RUN else 'live-r3.lock'),'a')
    try:
        try: fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError: row.update(build='skipped',reason='lock-held'); return
        # Restore current even if selection/fetch later fails or the heads are unchanged.
        if not DRY_RUN and (P/'current').exists():
            row['url']=activate(os.path.realpath(P/'current'))
        # gh's files field may be truncated; inspect complete paginated file lists for unmatched PRs.
        prs=json.loads(run(['gh','pr','list','--repo','compoundingtech/fractal','--state','open','--limit','1000','--json','number,headRefName,headRefOid,baseRefName,isDraft,files'],capture=True,timeout=120).stdout)
        manual=set(json.loads((L/'live-r3-drop.json').read_text())) if (L/'live-r3-drop.json').exists() else set()
        selected=[]
        for pr in prs:
            if select_pr(pr, row, manual): selected.append(pr)
        # Priority PRs first, then all open kit PRs, then newest first. A child
        # already carries its ancestors; merging old bases first defeats priority.
        rank = {n: i for i, n in enumerate(PRIORITY)}
        def priority(p):
            kit = any(f['path'].startswith('packages/fractal-ui/') for f in p['files'])
            return (0, rank[p['number']]) if p['number'] in rank else (1 if kit else 2, -p['number'])
        ordered=sorted(selected,key=priority)
        missing=REQUIRED-{p['number'] for p in ordered}
        for number in sorted(missing):
            merged=json.loads(run(['gh','pr','view',str(number),'--repo','compoundingtech/fractal','--json','number,headRefName,headRefOid,baseRefName,isDraft,files,state'],capture=True,timeout=120).stdout)
            if merged['state'] != 'MERGED':
                raise RuntimeError(f'required ui-design PR #{number} missing from selection and not merged')
            # GitHub retains pull/head refs after merge. Keep the required head
            # explicit even when it has left the open-PR query.
            ordered.append(merged)
            row['selection'].append({'pr':number,'reason':'required-merged','draft':False,'files':[f['path'] for f in merged['files']]})
        ordered.sort(key=priority)
        if pin:
            row['skipped'].extend({'pr':p['number'],'reason':'pinned-pending-reconciliation','files':[]} for p in ordered if p['number'] not in REQUIRED)
            ordered=[p for p in ordered if p['number'] in REQUIRED]
        if not C.exists():
            run(['git','clone','--no-hardlinks',SOURCE_CLONE,C]); git('remote','set-url','origin','https://github.com/compoundingtech/fractal.git')
        exclude=C/'.git/info/exclude'
        for pattern in ('/apps/fractal-web/private/','/apps/fractal-web/vendor/'):
            if pattern not in exclude.read_text():
                with exclude.open('a') as f: f.write('\n'+pattern+'\n')
        prepare_clone()
        git('fetch','origin','+refs/heads/main:refs/remotes/origin/main',*[f'+refs/pull/{p["number"]}/head:refs/remotes/origin/live-r3-pr-{p["number"]}' for p in ordered])
        row['main']=text('rev-parse','origin/main')
        for number, commit in LOCAL_HEAD_OVERRIDES.items():
            if any(p['number'] == number for p in ordered):
                git('fetch', S/'clones'/f'm-fix-{number}', f'+{commit}:refs/remotes/origin/live-r3-pr-{number}')
        ordered = [pr for pr in ordered if allow_wire_contract(pr, row)]
        private=PRIVATE_OVERLAY; private_hash=digest(private)
        heads=[{'pr':p['number'],'head':text('rev-parse',f'origin/live-r3-pr-{p["number"]}')} for p in ordered]
        # The recorded semantic resolution stack includes all required heads as
        # ancestors. Fast-forward PR updates merge above it automatically; a
        # rewritten head needs a refreshed, reviewed semantic resolution stack.
        stack_path=L/'live-r3-required-stack.json'
        stack=json.loads(stack_path.read_text()) if stack_path.exists() else None
        if stack:
            git('fetch',S/'clones'/stack['clone'],f'+{stack["sha"]}:refs/remotes/origin/live-r3-required-stack')
            if pin and pin['stack'] != stack['sha']:
                raise RuntimeError('pinned stack differs from the validated recorded stack')
            actual={str(h['pr']):h['head'] for h in heads if h['pr'] in REQUIRED}
            for number, previous in stack['heads'].items():
                current=actual[number]
                if current != previous:
                    if pin:
                        row.setdefault('pendingRequiredHeads',[]).append({'pr':int(number),'from':previous,'to':current})
                        continue
                    if git('merge-base','--is-ancestor',previous,current,capture=True,check=False).returncode:
                        raise RuntimeError(f'required PR #{number} was rewritten; refresh its recorded semantic resolution stack')
                    row.setdefault('requiredHeadUpdates',[]).append({'pr':int(number),'from':previous,'to':current})
            row['resolutionStack']={'sha':stack['sha'],'required':sorted(REQUIRED),'files':stack['files']}
            if pin:
                heads=[{'pr':h['pr'],'head':stack['heads'][str(h['pr'])]} for h in heads]
        base=pin['main'] if pin else row['main']
        REGISTRY_BASE = base
        signature=hashlib.sha256(json.dumps([INTEGRATOR_VERSION,base,heads,private_hash,sorted(manual),stack,pin]).encode()).hexdigest()
        state=L/'live-r3-state.json'
        if not DRY_RUN and state.exists() and json.loads(state.read_text())['signature']==signature and (P/'current').exists():
            old=json.loads(state.read_text()); row.update(sha=old['sha'],included=old['included'],skipped=old['skipped'],build='skipped',reason='unchanged',selectionComplete=True); return
        git('checkout','-B','live-r3',base)
        overlay=C/'apps/fractal-web/private'
        if not (C/'apps/fractal-web/vendor/utils/src/node/cli-build-identity.js').exists():
            shutil.copytree(VENDOR,C/'apps/fractal-web/vendor',symlinks=True,dirs_exist_ok=True)
        if overlay.exists(): shutil.rmtree(overlay)
        shutil.copytree(private,overlay,symlinks=True)
        config=overlay/'vite.config.ts'
        if not any('private-annotations' in f.read_text() for f in (C/'apps/fractal-web/src').rglob('DevBar.tsx')):
            config.write_text(config.read_text().replace('privateAnnotations(),','').replace(', privateAnnotations()', '').replace('privateAnnotations()', ''))
        # Main has no private extension hooks: retain its unchanged public boundary.
        builder=overlay/'build-live.mjs'
        builder.write_text(builder.read_text().replace("configFile: resolve(app, 'private/vite.config.ts')","configFile: resolve(app, 'vite.config.ts')"))
        row['buildProfile']='public-client/private-server'
        if stack:
            result=git('merge','--no-ff','--no-edit',stack['sha'],check=False)
            if result.returncode:
                resolve_additive()
                conflicts=text('diff','--name-only','--diff-filter=U').splitlines()
                if conflicts:
                    row['stackConflictFiles']=conflicts
                    git('merge','--abort')
                    raise RuntimeError(f'recorded required-PR resolution conflicts with main: {conflicts}')
                git('commit','--no-edit')
        retain_main_registry()
        checkpoints=[{'sha':text('rev-parse','HEAD'),'pr':None}]
        for p,head in zip(ordered,heads):
            result=git('merge','--no-ff','--no-edit',head['head'],check=False)
            if result.returncode:
                resolved=resolve_additive()
                conflicts=text('diff','--name-only','--diff-filter=U').splitlines()
                if conflicts:
                    git('merge','--abort',check=False)
                    if p['number'] in REQUIRED:
                        row['requiredConflict']={'pr':p['number'],'files':conflicts}
                        raise RuntimeError(f'required ui-design PR #{p["number"]} needs semantic resolution: {conflicts}')
                    row['skipped'].append({'pr':p['number'],'reason':'merge-conflict','files':conflicts,'autoResolved':resolved})
                    continue
                git('commit','--no-edit')
                row.setdefault('resolutions',[]).append({'pr':p['number'],'files':resolved})
            retain_main_registry()
            row['included'].append(head)
            checkpoints.append({**head,'sha':text('rev-parse','HEAD')})
        row['selectionComplete']=True
        if DRY_RUN:
            prepare_dependencies()
            row.update(sha=text('rev-parse','HEAD'),build='dry-run',reason='merged-and-regenerated-no-build-no-swap')
            return
        previous=os.path.realpath(P/'current') if (P/'current').exists() else None
        if not build_candidate(row,private_hash,checkpoints):
            atomic_json(state,{'signature':signature,'sha':row['sha'],'included':row['included'],'skipped':row['skipped'],'pinnedBase':row['pinnedBase']})
            return
        row['url']=activate(previous)
        row['build']='built'
        state.write_text(json.dumps({'signature':signature,'sha':row['sha'],'included':row['included'],'skipped':row['skipped'],'pinnedBase':row['pinnedBase']}))
    except Exception as e:
        row['error']=str(e)
        if isinstance(e,subprocess.CalledProcessError):
            row['detail']=((e.stdout or '') + '\n' + (e.stderr or '')).strip()[-4000:]
        print('tick failed:',row['error'],row.get('detail',''),flush=True)
    finally:
        observe_served(row)
        row['ms']=round((time.monotonic()-start)*1000)
        row['timingsMs']['other']=max(0,row['ms']-sum(row['timingsMs'].values()))
        publish_skips(row)
        TIMINGS=None
        with (L/('live-r3-dry-run.jsonl' if DRY_RUN else 'live-r3.jsonl')).open('a') as f: f.write(json.dumps(row,separators=(',',':'))+'\n')
        print(json.dumps(row),flush=True)
        lock.close()
if __name__=='__main__':
    import sys
    if '--once' in sys.argv or DRY_RUN: tick()
    else:
        while True:
            start=time.monotonic(); tick(); time.sleep(max(0,300-(time.monotonic()-start)))
