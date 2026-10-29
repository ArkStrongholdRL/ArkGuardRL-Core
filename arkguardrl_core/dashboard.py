"""Read-only local dashboard for native traces and JSON research reports."""
import argparse
from collections import OrderedDict
import gzip
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import mimetypes
import os
from pathlib import Path
import threading
import shutil
from urllib.parse import parse_qs, urlsplit, unquote

STATIC = Path(__file__).with_name('dashboard_static')
MAX_JSON = 32 * 1024**2
MAX_LINE = 8 * 1024**2
MAX_SCAN = 512 * 1024**2


def read_json(path):
    with path.open('rb') as stream:
        raw = stream.read(MAX_JSON + 1)
    if len(raw) > MAX_JSON:
        raise ValueError('JSON report exceeds 32 MiB; use JSONL for full trajectories')
    return json.loads(raw)


def safe_json(value):
    if isinstance(value, float) and not math.isfinite(value):
        return None
    if isinstance(value, dict):
        return {k: safe_json(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [safe_json(v) for v in value]
    return value


class Store:
    def __init__(self, root, engine_root=None, vendor_root=None):
        self.root = Path(root).resolve(strict=True)
        if not self.root.is_dir():
            raise ValueError('Results root must be a directory')
        self.cache = OrderedDict()
        self.lock = threading.Lock()
        self.catalog = {'names': {}, 'stages': {}}
        self.engine_root = Path(engine_root).resolve(strict=True) if engine_root else None
        self.vendor_root = (Path(vendor_root).resolve(strict=True) if vendor_root else
                            self.engine_root / 'public' / 'vendor' if self.engine_root else None)
        self.catalog['renderer'] = dict(enabled=self.engine_root is not None,
            vendor=bool(self.vendor_root and (self.vendor_root / 'pixi.min.js').is_file()),
            art=bool(self.engine_root and (self.engine_root / 'public' / 'assets').is_dir()))
        if engine_root:
            data = Path(engine_root) / 'data'
            for name in ('chess', 'items', 'tokens', 'bonds'):
                path = data / f'{name}.json'
                if path.is_file():
                    for key, value in read_json(path).items():
                        if isinstance(value, dict):
                            self.catalog['names'][key] = value.get('name', key)
            path = data / 'stages.json'
            if path.is_file():
                for key, stage in read_json(path).items():
                    self.catalog['stages'][key] = {k: stage[k] for k in ('name', 'rows', 'tiles', 'size') if k in stage}

    def path(self, relative):
        path = (self.root / relative).resolve(strict=True)
        if not path.is_relative_to(self.root) or not path.is_file():
            raise ValueError('File is outside the results directory')
        if not path.name.endswith(('.json', '.jsonl', '.jsonl.gz', '.ndjson', '.ndjson.gz')):
            raise ValueError('Only JSON reports and JSONL trajectories are supported')
        return path

    def files(self):
        entries = []
        excluded = {'.git', '.venv', 'node_modules', '__pycache__'}
        for directory, dirs, files in os.walk(self.root, followlinks=False):
            dirs[:] = sorted(d for d in dirs if d not in excluded and not Path(directory, d).is_symlink())
            for name in sorted(files):
                if not name.endswith(('.json', '.jsonl', '.jsonl.gz', '.ndjson', '.ndjson.gz')):
                    continue
                path = Path(directory, name)
                if path.is_symlink():
                    continue
                try:
                    stat = path.stat()
                except FileNotFoundError:
                    continue
                entries.append(dict(path=str(path.relative_to(self.root)), size=stat.st_size, modified=stat.st_mtime))
                if len(entries) >= 10000:
                    return dict(files=sorted(entries, key=lambda e: e['modified'], reverse=True), capped=True)
        return dict(files=sorted(entries, key=lambda e: e['modified'], reverse=True), capped=False)

    def document(self, relative):
        path = self.path(relative)
        stat = path.stat()
        signature = (stat.st_mtime_ns, stat.st_size, stat.st_ino)
        with self.lock:
            previous = self.cache.get(path)
            if previous and previous[0] == signature:
                self.cache.move_to_end(path)
                return previous[1]
            is_trace = path.name.endswith(('.jsonl', '.ndjson', '.gz'))
            incremental = (previous[1] if previous and is_trace and path.suffix != '.gz'
                           and previous[0][2] == stat.st_ino and previous[0][1] <= stat.st_size
                           and not previous[1].get('_capped') else None)
            doc = self.trace(path, incremental) if is_trace else self.report(path)
            doc['path'] = relative
            doc['modified'] = stat.st_mtime
            self.cache[path] = (signature, doc)
            self.cache.move_to_end(path)
            while len(self.cache) > 4:
                self.cache.popitem(last=False)
            return doc

    def trace(self, path, previous=None):
        timeline = list(previous['timeline']) if previous else []
        offsets = list(previous['_offsets']) if previous else []
        header, footer = (previous['header'], previous['footer']) if previous else ({}, {})
        warning, capped = None, False
        next_offset = previous['_next'] if previous else 0
        opener = gzip.open if path.suffix == '.gz' else open
        with opener(path, 'rb') as stream:
            stream.seek(next_offset)
            while True:
                offset = stream.tell()
                if offset >= MAX_SCAN or len(timeline) >= 50000:
                    warning = '显示上限：512 MiB 解压数据 / 50,000 帧'
                    capped = True
                    break
                try:
                    line = stream.readline(MAX_LINE + 1)
                except (EOFError, OSError):
                    warning = '压缩流尚未结束或已损坏；显示已读取部分'
                    break
                if not line:
                    break
                if len(line) > MAX_LINE:
                    raise ValueError('Trajectory row exceeds 8 MiB')
                if not line.endswith(b'\n'):
                    warning = '末行尚未写完；显示完整行'
                    break
                try:
                    envelope = json.loads(line)
                except (ValueError, UnicodeError) as exc:
                    raise ValueError(f'Invalid trajectory JSON at byte {offset}') from exc
                if not isinstance(envelope, dict):
                    raise ValueError(f'Trajectory row is not an object at byte {offset}')
                record = envelope.get('record', envelope)
                if not isinstance(record, dict):
                    raise ValueError(f'Trajectory record is not an object at byte {offset}')
                next_offset = stream.tell()
                kind = record.get('kind')
                if kind == 'header':
                    header = record
                if kind == 'footer':
                    footer = record
                state = record.get('after') or record.get('state')
                if isinstance(state, dict):
                    own = state.get('own', {})
                    timeline.append(dict(round=state.get('round'), phase=state.get('phase'),
                        hp=own.get('lp'), funds=own.get('funds'), kind=kind,
                        action=record.get('action'), accepted=record.get('accepted'), reward=record.get('reward')))
                    offsets.append(offset)
        return dict(format='native', precision='逐步状态快照', header=header, footer=footer,
                    status=footer.get('status', '未闭合记录'), warning=warning, timeline=timeline,
                    _offsets=offsets, _next=next_offset, _capped=capped)

    def report(self, path):
        raw = read_json(path)
        if not isinstance(raw, dict):
            return dict(format='metrics', precision='JSON 数据', status='报告', timeline=[], raw=raw)
        timeline = []
        ready = raw.get('ready')
        if isinstance(ready, list):
            timeline = [dict(round=r.get('round'), phase='READY', hp=r.get('hp'), funds=r.get('funds'),
                             hp_loss=r.get('hp_loss'), kind='ready') for r in ready if isinstance(r, dict)]
        elif isinstance(raw.get('own'), dict):
            timeline = [dict(round=raw.get('round'), phase='DECISION', hp=raw.get('hp'), funds=raw.get('funds'), kind='case')]
        if raw.get('truncated'):
            status = '预算截断'
        elif raw.get('natural'):
            status = '自然结束'
        else:
            status = raw.get('status', raw.get('state', '报告'))
        return dict(format='report' if timeline else 'metrics', precision='每轮 Ready 快照' if ready else '决策快照' if timeline else '指标 / 汇总',
                    status=status, timeline=timeline, raw=raw)

    def frame(self, relative, index):
        doc = self.document(relative)
        if not 0 <= index < len(doc['timeline']):
            raise ValueError('Frame index out of range')
        if doc['format'] == 'native':
            path = self.path(relative)
            opener = gzip.open if path.suffix == '.gz' else open
            with opener(path, 'rb') as stream:
                stream.seek(doc['_offsets'][index])
                envelope = json.loads(stream.readline(MAX_LINE + 1))
            record = envelope.get('record', envelope)
            return dict(state=record.get('after') or record.get('state'), record=record)
        raw = doc['raw']
        if isinstance(raw.get('ready'), list):
            ready = [r for r in raw['ready'] if isinstance(r, dict)][index]
            return dict(state=dict(round=ready.get('round'), phase='READY', own={**ready, 'lp': ready.get('hp')}, public={'stageId': raw.get('stage')}),
                        record=ready, events=[e for e in raw.get('events', []) if e.get('round') == ready.get('round')])
        return dict(state=dict(round=raw.get('round'), phase='DECISION', own=raw['own'], public={}), record=raw)


class Handler(BaseHTTPRequestHandler):
    def engine_file(self, request_path):
        store = self.server.store
        if store.engine_root is None:
            raise ValueError('Native game rendering requires --engine-root')
        prefix, _, relative = unquote(request_path).lstrip('/').partition('/')
        mounts = {name: store.engine_root / 'public' / name for name in ('js', 'assets', 'fonts')}
        mounts.update(data=store.engine_root / 'data', shared=store.engine_root / 'shared', vendor=store.vendor_root)
        root = mounts.get(prefix)
        if root is None:
            raise FileNotFoundError(request_path)
        root = root.resolve()
        path = (root / relative).resolve(strict=True)
        if not path.is_relative_to(root) or not path.is_file():
            raise ValueError('Resource outside mounted game directory')
        # Stream external resources in place; never copy engine code or art into Core.
        with path.open('rb') as stream:
            self.send_response(200)
            self.send_header('Content-Type', mimetypes.guess_type(path.name)[0] or 'application/octet-stream')
            self.send_header('Content-Length', str(os.fstat(stream.fileno()).st_size))
            self.send_header('Cache-Control', 'no-cache')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.end_headers()
            shutil.copyfileobj(stream, self.wfile, length=256 * 1024)

    def send(self, value, code=200, content_type='application/json; charset=utf-8'):
        body = value if isinstance(value, bytes) else json.dumps(safe_json(value), ensure_ascii=False, allow_nan=False).encode()
        self.send_response(code)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Content-Security-Policy', "default-src 'self'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data: blob:; worker-src 'self' blob:; object-src 'none'; frame-ancestors 'none'")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        url = urlsplit(self.path)
        query = parse_qs(url.query)
        try:
            host = self.headers.get('Host', '').split(':', 1)[0]
            if host not in ('127.0.0.1', 'localhost'):
                self.send({'error': 'Localhost access only'}, 403)
                return
            store = self.server.store
            if url.path == '/api/files':
                self.send(store.files())
            elif url.path == '/api/catalog':
                self.send(store.catalog)
            elif url.path == '/api/document':
                doc = store.document(query.get('path', [''])[0])
                self.send({k: v for k, v in doc.items() if not k.startswith('_')})
            elif url.path == '/api/frame':
                self.send(store.frame(query.get('path', [''])[0], int(query.get('index', ['0'])[0])))
            elif url.path in ('/', '/app.js', '/style.css', '/game-view.js'):
                filename, mime = {'/': ('index.html', 'text/html; charset=utf-8'), '/app.js': ('app.js', 'text/javascript; charset=utf-8'), '/game-view.js': ('game-view.js', 'text/javascript; charset=utf-8'), '/style.css': ('style.css', 'text/css; charset=utf-8')}[url.path]
                self.send((STATIC / filename).read_bytes(), content_type=mime)
            elif (url.path == '/data/assets.json' and store.engine_root and
                  not (store.engine_root / 'public' / 'assets').is_dir()):
                self.send(dict(version=1, chars={}, enemies={}, tokens={}, stats={}))
            elif (url.path in ('/data/local-assets.json', '/data/local-fx.json') and store.engine_root and
                  not (store.engine_root / url.path.lstrip('/')).is_file()):
                self.send(dict(version=1, source='none', groups={}))
            elif url.path.startswith(('/js/', '/data/', '/shared/', '/vendor/', '/assets/', '/fonts/')):
                self.engine_file(url.path)
            else:
                self.send({'error': 'Not found'}, 404)
        except FileNotFoundError:
            self.send({'error': '文件不存在'}, 404)
        except (ValueError, KeyError, TypeError, OSError) as exc:
            self.send({'error': str(exc)}, 400)

    def log_message(self, *_):
        pass


def serve(root, engine_root=None, host='127.0.0.1', port=8765, vendor_root=None):
    store = Store(root, engine_root, vendor_root)
    with ThreadingHTTPServer((host, port), Handler) as server:
        server.store = store
        print(f'Dashboard: http://{host}:{server.server_port}  results={store.root}', flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results-root', type=Path, required=True)
    parser.add_argument('--engine-root', type=Path, help='External game installation for native rendering')
    parser.add_argument('--vendor-root', type=Path, help='Optional existing game public/vendor directory')
    parser.add_argument('--port', type=int, default=8765)
    args = parser.parse_args()
    serve(args.results_root, args.engine_root, port=args.port, vendor_root=args.vendor_root)


if __name__ == '__main__':
    main()
