"""VM-only socket2-to-minimap latency probe; no requests run while idle."""
import threading

class MinimapLatency:
    def __init__(self, instance, cli, log):
        self.records = []
        self.events = []
        self.cv = threading.Condition()
        self.process = subprocess.Popen([cli, "--instance", instance.name, "waybar"],
            stdout=subprocess.PIPE, stderr=log, start_new_session=True)
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.connect(str(instance / ".socket2.sock"))
        def records():
            for line in self.process.stdout:
                with self.cv:
                    self.records.append((time.monotonic(), json.loads(line)))
                    self.cv.notify_all()
        def events():
            with self.socket.makefile("rb") as stream:
                for line in stream:
                    with self.cv:
                        self.events.append((time.monotonic(), line.decode().strip()))
                        self.cv.notify_all()
        threading.Thread(target=records, daemon=True).start()
        threading.Thread(target=events, daemon=True).start()
        self.results = {}
        with self.cv:
            assert self.cv.wait_for(lambda: self.records, 10), "minimap initial line"

    def expect(self, name, action, predicate, event=None):
        # Let preceding fixture mutations settle before measuring this action.
        time.sleep(.08)
        start = time.monotonic()
        action()
        with self.cv:
            def match():
                return next(((t, r) for t, r in self.records if t >= start and predicate(r)), None)
            assert self.cv.wait_for(match, 10), (name, self.records[-3:], self.events[-10:])
            timestamp, record = match()
            if event:
                assert self.cv.wait_for(lambda: any(t >= start and line.startswith(event + ">>") for t,line in self.events), 10), name
                start = next(t for t,line in self.events if t >= start and line.startswith(event + ">>"))
            elapsed = (timestamp-start)*1000
            # Reader scheduling can timestamp simultaneously-ready sockets in
            # either order. It cannot inflate the permitted 50ms upper bound.
            self.results[name] = max(0, elapsed)
            assert elapsed < 50, (name, elapsed, self.events[-10:])
        print("PASS minimap latency", name, round(max(0,elapsed),2), "ms", flush=True)
        return record

    def close(self):
        self.process.terminate()
        self.process.wait(timeout=10)
        self.socket.shutdown(socket.SHUT_RDWR)
        self.socket.close()
