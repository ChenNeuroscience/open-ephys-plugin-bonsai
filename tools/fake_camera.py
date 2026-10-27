"""
Fake camera for the Rust tracker: renders an animal with two head LEDs running around an arena
and streams raw grayscale frames over TCP. Standard library only, no pip installs needed.

    python3 tools/fake_camera.py [--port 27030] [--fps 50] [--width 640] [--height 480]

Then start acquisition in Open Ephys (or `cargo run --release --bin tracker-dev` in tracker/).
The tracker connects as a client; when it disconnects we wait for the next one.

Wire format per frame, little-endian (must match tracker/src/camera/fake.rs):
    b"FRM1", u32 width, u32 height, u64 frame_id, f64 timestamp_seconds, width*height uint8 pixels
"""
import argparse
import math
import random
import socket
import struct
import time

HEADER = struct.Struct("<4sIIQd")


def make_background(w, h):
    """Dim floor, slightly brighter arena disc, and a bright lamp reflection outside it (for testing the mask)."""
    bg = bytearray(b"\x10" * (w * h))
    cx, cy, r2 = w / 2, h / 2, (min(w, h) * 0.45) ** 2
    for y in range(h):
        row = y * w
        for x in range(w):
            if (x - cx) ** 2 + (y - cy) ** 2 < r2:
                bg[row + x] = 0x30
    draw_disc(bg, w, h, w - 25, 25, 10, 0xFF)
    return bg


def draw_disc(buf, w, h, cx, cy, r, value):
    cx, cy = int(cx), int(cy)
    for y in range(max(0, cy - r), min(h, cy + r + 1)):
        dx = int(math.sqrt(max(0, r * r - (y - cy) ** 2)))
        x0, x1 = max(0, cx - dx), min(w - 1, cx + dx)
        if x0 <= x1:
            buf[y * w + x0 : y * w + x1 + 1] = bytes([value]) * (x1 - x0 + 1)


class Animal:
    """Same idea as dummy-bonsai.py: a random walk that bounces off the arena wall."""

    def __init__(self, w, h):
        self.cx, self.cy, self.radius = w / 2, h / 2, min(w, h) * 0.45 - 20
        self.x, self.y, self.theta, self.v = self.cx, self.cy, 0.0, 3.0
        self.big_on = self.small_on = True

    def step(self):
        self.theta += random.uniform(-0.25, 0.25)
        self.v = min(max(0.0, self.v + random.uniform(-0.5, 0.5)), 6.0)
        nx, ny = self.x + self.v * math.cos(self.theta), self.y + self.v * math.sin(self.theta)
        if math.hypot(nx - self.cx, ny - self.cy) > self.radius:
            self.theta = math.atan2(self.cy - self.y, self.cx - self.x) + random.uniform(-0.5, 0.5)
        else:
            self.x, self.y = nx, ny
        r = random.random()  # occasional LED occlusion, as in dummy-bonsai.py
        if r < 0.005:
            self.big_on = self.small_on = False
        elif r < 0.01:
            self.small_on = False
        elif r < 0.05:
            self.big_on = self.small_on = True

    def draw(self, buf, w, h):
        if self.big_on:
            draw_disc(buf, w, h, self.x, self.y, 5, 0xFF)
        if self.small_on:
            back = self.theta + math.pi
            draw_disc(buf, w, h, self.x + 18 * math.cos(back), self.y + 18 * math.sin(back), 3, 0xF0)


def serve_client(conn, args, background):
    animal = Animal(args.width, args.height)
    interval = 1.0 / args.fps
    next_due = time.monotonic()
    frame_id = 0
    sent_at = time.monotonic()
    while True:
        animal.step()
        frame = bytearray(background)
        animal.draw(frame, args.width, args.height)
        conn.sendall(HEADER.pack(b"FRM1", args.width, args.height, frame_id, time.time()) + frame)
        frame_id += 1
        if frame_id % (int(args.fps) * 5) == 0:
            now = time.monotonic()
            print(f"  sent {frame_id} frames ({int(args.fps) * 5 / (now - sent_at):.1f} fps)")
            sent_at = now
        next_due += interval
        delay = next_due - time.monotonic()
        if delay > 0:
            time.sleep(delay)
        else:
            next_due = time.monotonic()  # fell behind; don't try to catch up in a burst


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=27030)
    p.add_argument("--fps", type=float, default=50)
    p.add_argument("--width", type=int, default=640)
    p.add_argument("--height", type=int, default=480)
    args = p.parse_args()

    print("rendering background...")
    background = make_background(args.width, args.height)
    with socket.create_server((args.host, args.port)) as server:
        while True:
            print(f"fake camera {args.width}x{args.height}@{args.fps}fps waiting on {args.host}:{args.port}")
            conn, addr = server.accept()
            print(f"client connected from {addr}")
            with conn:
                conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                try:
                    serve_client(conn, args, background)
                except (BrokenPipeError, ConnectionResetError):
                    print("client disconnected")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
