//! avplayer.m — Native macOS AVPlayer-backed video player for rpick.
//!
//! Provides an NSWindow with AVPlayerView, native controls (progress
//! slider, time label, play/pause/skip buttons, file counter), help
//! overlay, and fullscreen support. Key events are enqueued into a
//! Rust-accessible ring buffer rather than calling back across FFI.

#import <Cocoa/Cocoa.h>
#import <AVKit/AVKit.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>

// ── Thread-safe key event ring buffer (polled by Rust) ──────────────
#define KEY_QUEUE_CAPACITY 64
static int            g_key_queue[KEY_QUEUE_CAPACITY];
static volatile int   g_key_write = 0;
static volatile int   g_key_read  = 0;
static NSLock        *g_key_lock  = nil;

// Called from the Cocoa event handlers on the main thread; Rust polls it.
static void enqueue_key(int key) {
    [g_key_lock lock];
    int next = (g_key_write + 1) % KEY_QUEUE_CAPACITY;
    if (next != g_key_read) {          // not full
        g_key_queue[g_key_write] = key;
        g_key_write = next;
    }
    [g_key_lock unlock];
}

int avplayer_poll_key(void) {
    int key = -1;
    [g_key_lock lock];
    if (g_key_read != g_key_write) {
        key = g_key_queue[g_key_read];
        g_key_read = (g_key_read + 1) % KEY_QUEUE_CAPACITY;
    }
    [g_key_lock unlock];
    return key;
}

// ── Global UI state ─────────────────────────────────────────────────
static NSWindow          *g_window           = nil;
static AVPlayerView      *g_playerView       = nil;
static AVPlayer          *g_player           = nil;
static id                 g_timeObserver     = nil;
static volatile BOOL      g_sliderInUse      = NO;
static volatile BOOL      g_videoFinished    = NO;
static volatile BOOL      g_helpVisible      = NO;
static volatile int       g_videoGeneration  = 0;

// Controls
static NSSlider          *g_progressBar      = nil;
static NSTextField       *g_timeLabel        = nil;
static NSTextField       *g_fileLabel        = nil;
static NSButton          *g_playBtn          = nil;
static NSButton          *g_skipBackBtn      = nil;
static NSButton          *g_skipFwdBtn       = nil;
static NSView            *g_helpOverlay      = nil;

// ── Layout constants ────────────────────────────────────────────────
static const CGFloat kBarHeight     = 42;   // control bar height
static const CGFloat kProgressRowH  = 24;   // progress row height
static const CGFloat kPadding       = 12;

// ── Color helpers ───────────────────────────────────────────────────
static NSColor* barBg(void)          { return [NSColor colorWithWhite:0.12 alpha:0.95]; }
static NSColor* dimTextColor(void)   { return [NSColor colorWithWhite:0.6  alpha:1.0]; }
static NSColor* timeText(void)       { return [NSColor colorWithWhite:0.92 alpha:1.0]; }

// ── Custom slider cell ──────────────────────────────────────────────
@interface DarkSliderCell : NSSliderCell
@end
@implementation DarkSliderCell
- (void)drawBarInside:(NSRect)rect flipped:(BOOL)flipped {
    NSRect trackRect = rect;
    trackRect.size.height = 4;
    trackRect.origin.y = rect.origin.y + (rect.size.height - 4) / 2;
    // background track
    [[NSColor colorWithWhite:0.235 alpha:1] setFill];
    [[NSBezierPath bezierPathWithRoundedRect:trackRect xRadius:2 yRadius:2] fill];
    // filled portion
    double frac = (self.doubleValue - self.minValue) / (self.maxValue - self.minValue);
    if (frac > 0) {
        NSRect fill = trackRect;
        fill.size.width = trackRect.size.width * frac;
        [[NSColor whiteColor] setFill];
        [[NSBezierPath bezierPathWithRoundedRect:fill xRadius:2 yRadius:2] fill];
    }
}
- (void)drawKnob:(NSRect)knobRect { /* hide default knob */ }
@end

// ── Key-capturing content view ──────────────────────────────────────
@interface KeyView : NSView
@end
@implementation KeyView
- (BOOL)acceptsFirstResponder { return YES; }
- (BOOL)canBecomeKeyView      { return YES; }

- (void)keyDown:(NSEvent *)event {
    NSString *chars = event.charactersIgnoringModifiers;
    if (chars.length == 0) return;
    unichar ch = [chars characterAtIndex:0];

    // Extended keys
    switch (ch) {
        case NSLeftArrowFunctionKey:  enqueue_key(81);      return;
        case NSRightArrowFunctionKey: enqueue_key(83);      return;
        case NSUpArrowFunctionKey:    enqueue_key(82);      return;
        case NSDownArrowFunctionKey:  enqueue_key(84);      return;
        case 27:  if ([g_window styleMask] & NSWindowStyleMaskFullScreen) {
                      [g_window toggleFullScreen:nil];
                  }
                  enqueue_key(27);
                  return;
        default: break;
    }

    // Printable ASCII
    if (ch < 128) {
        unichar lower = ch;
        if (ch >= 'A' && ch <= 'Z') lower = ch + 32; // lowercase

        // Handle keys that ObjC handles directly
        switch (ch) {
            case ' ':  // space → play/pause
                [self togglePlayPause];
                enqueue_key(' ');
                return;
            case '?':  // toggle help
                g_helpVisible = !g_helpVisible;
                g_helpOverlay.hidden = !g_helpVisible;
                [g_helpOverlay setNeedsDisplay:YES];
                return;
            case 'F':  // UPPERCASE F → fullscreen toggle
                [g_window toggleFullScreen:nil];
                return;
        }

        // Enqueue the *lowercase* key code so Rust handlers can match
        // 'f', 'g', 'd', 'q', 'n', 'a', 'o', 'c', 'v', 'e', 'w', 't', 'u', 'z'
        enqueue_key((int)lower);
    }
}

// ── Help overlay (drawn with AppKit text) ───────────────────────────
- (void)drawHelpOverlayIfNeeded {
    if (!g_helpVisible) return;
    [[NSColor colorWithWhite:0.0 alpha:0.82] setFill];
    NSRectFill(self.bounds);

    NSDictionary *titleAttr = @{
        NSFontAttributeName: [NSFont systemFontOfSize:18 weight:NSFontWeightBold],
        NSForegroundColorAttributeName: [NSColor colorWithRed:1 green:0.9 blue:0.3 alpha:1] };
    NSDictionary *keyAttr = @{
        NSFontAttributeName: [NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightBold],
        NSForegroundColorAttributeName: [NSColor whiteColor] };
    NSDictionary *descAttr = @{
        NSFontAttributeName: [NSFont systemFontOfSize:12 weight:NSFontWeightRegular],
        NSForegroundColorAttributeName: [NSColor colorWithWhite:0.7 alpha:1] };

    struct { char *key; char *desc; } shortcuts[] = {
        {"Space",  "Play / Pause"},
        {"←  →",   "Seek ±30s  (c/v)"},
        {"↑  ↓",   "Prev / Next video"},
        {"g",      "Mark Good → ../Good"},
        {"f",      "Mark Fine → ../Fine"},
        {"d",      "Trash"},
        {"n / a",  "Next video"},
        {"o",      "Reveal in Finder"},
        {"e",      "Jump to end (95%)"},
        {"w",      "Add Purple tag"},
        {"t",      "OCR text detection"},
        {"u / z",  "Undo"},
        {"F",      "Toggle fullscreen"},
        {"q",      "Quit"},
        {"?",      "This help"},
    };
    int count = sizeof(shortcuts)/sizeof(shortcuts[0]);
    CGFloat startY = self.bounds.size.height - 60;
    CGFloat colW   = self.bounds.size.width / 2;

    [@"KEYBOARD SHORTCUTS" drawAtPoint:NSMakePoint(colW-120, startY) withAttributes:titleAttr];
    startY -= 36;
    int half = (count+1)/2;
    for (int i = 0; i < count; i++) {
        int col = (i < half) ? 0 : 1;
        int row = (i < half) ? i : (i - half);
        CGFloat x = 30 + col * colW;
        CGFloat y = startY - row * 25;
        [@(shortcuts[i].key)  drawAtPoint:NSMakePoint(x, y)      withAttributes:keyAttr];
        [@(shortcuts[i].desc) drawAtPoint:NSMakePoint(x+80, y)  withAttributes:descAttr];
    }
}

- (void)drawRect:(NSRect)r {
    [[NSColor blackColor] setFill];
    NSRectFill(r);
    [self drawHelpOverlayIfNeeded];
}

- (void)mouseDown:(NSEvent *)event {
    if (g_helpVisible) {
        g_helpVisible = NO;
        g_helpOverlay.hidden = YES;
        return;
    }
}

// ── Play/Pause toggle ────────────────────────────────────────────
- (void)togglePlayPause {
    if (!g_player) return;
    if (g_player.rate > 0) {
        [g_player pause];
        [self setPlaying:NO];
    } else {
        [g_player play];
        [self setPlaying:YES];
    }
}

- (void)setPlaying:(BOOL)playing {
    NSString *name = playing ? @"pause.fill" : @"play.fill";
    NSImage *img = [NSImage imageWithSystemSymbolName:name accessibilityDescription:nil];
    img = [img imageWithSymbolConfiguration:
           [NSImageSymbolConfiguration configurationWithPointSize:18 weight:NSFontWeightMedium]];
    g_playBtn.image = img;
}
@end

// ── SF Symbol button factory ─────────────────────────────────────────
static NSButton* makeSymbolBtn(NSString *symbol, CGFloat size, SEL action, id target) {
    NSImage *img = [NSImage imageWithSystemSymbolName:symbol accessibilityDescription:nil];
    img = [img imageWithSymbolConfiguration:
           [NSImageSymbolConfiguration configurationWithPointSize:size weight:NSFontWeightMedium]];
    NSButton *btn = [NSButton buttonWithImage:img target:target action:action];
    btn.bordered = NO;
    btn.contentTintColor = dimTextColor();
    btn.translatesAutoresizingMaskIntoConstraints = NO;
    return btn;
}

// ── Window delegate (quit on close) ──────────────────────────────────
@interface PlayerDelegate : NSObject <NSWindowDelegate>
@end
@implementation PlayerDelegate
- (BOOL)windowShouldClose:(NSWindow *)sender {
    enqueue_key('q');
    return NO; // let Rust handle quitting
}
@end

// ═══════════════════════════════════════════════════════════════════════
// C API exported for Rust FFI
// ═══════════════════════════════════════════════════════════════════════

void avplayer_init(void) {
    g_key_lock = [[NSLock alloc] init];

    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    // Basic menu so Cmd+Q works
    NSMenu *menubar = [[NSMenu alloc] init];
    NSMenuItem *appItem = [[NSMenuItem alloc] init];
    [menubar addItem:appItem];
    NSMenu *appMenu = [[NSMenu alloc] init];
    [appMenu addItemWithTitle:@"Quit rpick" action:@selector(terminate:) keyEquivalent:@"q"];
    appItem.submenu = appMenu;
    [NSApp setMainMenu:menubar];

    // ── Window ────────────────────────────────────────────────────────
    NSRect frame = NSMakeRect(0, 0, 960, 600);
    g_window = [[NSWindow alloc]
        initWithContentRect:frame
        styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                    NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable)
        backing:NSBackingStoreBuffered defer:NO];
    g_window.title = @"rpick";
    g_window.backgroundColor = [NSColor blackColor];
    g_window.titlebarAppearsTransparent = YES;
    g_window.titleVisibility = NSWindowTitleHidden;
    g_window.minSize = NSMakeSize(480, 320);
    g_window.delegate = [[PlayerDelegate alloc] init];
    [g_window center];

    NSView *content = g_window.contentView;
    content.wantsLayer = YES;

    // ── AVPlayerView ──────────────────────────────────────────────────
    g_playerView = [[AVPlayerView alloc] init];
    g_playerView.translatesAutoresizingMaskIntoConstraints = NO;
    g_playerView.controlsStyle = AVPlayerViewControlsStyleNone;
    g_playerView.wantsLayer = YES;
    g_playerView.layer.backgroundColor = [[NSColor blackColor] CGColor];

    // ── Key-capturing content (replaces stock contentView's event handling) ──
    KeyView *keyView = [[KeyView alloc] init];
    keyView.translatesAutoresizingMaskIntoConstraints = NO;
    keyView.wantsLayer = YES;
    [keyView addSubview:g_playerView];
    [content addSubview:keyView];

    [NSLayoutConstraint activateConstraints:@[
        [g_playerView.leadingAnchor constraintEqualToAnchor:keyView.leadingAnchor],
        [g_playerView.trailingAnchor constraintEqualToAnchor:keyView.trailingAnchor],
        [g_playerView.topAnchor constraintEqualToAnchor:keyView.topAnchor],
        [g_playerView.bottomAnchor constraintEqualToAnchor:keyView.bottomAnchor],
    ]];

    // ── Progress bar row ──────────────────────────────────────────────
    NSView *progressRow = [[NSView alloc] init];
    progressRow.translatesAutoresizingMaskIntoConstraints = NO;
    progressRow.wantsLayer = YES;
    progressRow.layer.backgroundColor = [barBg() CGColor];
    [content addSubview:progressRow];

    g_progressBar = [[NSSlider alloc] init];
    g_progressBar.translatesAutoresizingMaskIntoConstraints = NO;
    g_progressBar.minValue = 0;
    g_progressBar.maxValue = 1000;
    g_progressBar.doubleValue = 0;
    g_progressBar.continuous = YES;
    g_progressBar.target = keyView; // handled by slider action set below
    DarkSliderCell *cell = [[DarkSliderCell alloc] init];
    [g_progressBar setCell:cell];
    // re-set target after setCell wipes it
    g_progressBar.target = keyView;
    [progressRow addSubview:g_progressBar];

    // ── Control bar ───────────────────────────────────────────────────
    NSView *ctrlBar = [[NSView alloc] init];
    ctrlBar.translatesAutoresizingMaskIntoConstraints = NO;
    ctrlBar.wantsLayer = YES;
    ctrlBar.layer.backgroundColor = [barBg() CGColor];
    [content addSubview:ctrlBar];

    g_skipBackBtn = makeSymbolBtn(@"gobackward.30", 14, nil, keyView);
    g_playBtn     = makeSymbolBtn(@"play.fill",     18, nil, keyView);
    g_skipFwdBtn  = makeSymbolBtn(@"goforward.30",  14, nil, keyView);
    [ctrlBar addSubview:g_skipBackBtn];
    [ctrlBar addSubview:g_playBtn];
    [ctrlBar addSubview:g_skipFwdBtn];

    // Time label
    g_timeLabel = [NSTextField labelWithString:@"0:00 / 0:00"];
    g_timeLabel.translatesAutoresizingMaskIntoConstraints = NO;
    g_timeLabel.font = [NSFont monospacedDigitSystemFontOfSize:14 weight:NSFontWeightMedium];
    g_timeLabel.textColor = timeText();
    [ctrlBar addSubview:g_timeLabel];

    // File counter
    g_fileLabel = [NSTextField labelWithString:@""];
    g_fileLabel.translatesAutoresizingMaskIntoConstraints = NO;
    g_fileLabel.font = [NSFont monospacedDigitSystemFontOfSize:10 weight:NSFontWeightRegular];
    g_fileLabel.textColor = dimTextColor();
    [ctrlBar addSubview:g_fileLabel];

    // ── Constraints ───────────────────────────────────────────────────
    NSDictionary *views = @{@"key":keyView, @"prog":progressRow, @"bar":ctrlBar};
    NSDictionary *mets = @{@"progH":@(kProgressRowH), @"barH":@(kBarHeight)};

    for (NSString *name in @[@"key", @"prog", @"bar"]) {
        [content addConstraints:[NSLayoutConstraint
            constraintsWithVisualFormat:[NSString stringWithFormat:@"H:|[%@]|", name]
            options:0 metrics:nil views:views]];
    }
    [content addConstraints:[NSLayoutConstraint
        constraintsWithVisualFormat:@"V:|[key][prog(progH)][bar(barH)]|"
        options:0 metrics:mets views:views]];

    // Progress slider fills progress row
    [NSLayoutConstraint activateConstraints:@[
        [g_progressBar.leadingAnchor constraintEqualToAnchor:progressRow.leadingAnchor constant:kPadding],
        [g_progressBar.trailingAnchor constraintEqualToAnchor:progressRow.trailingAnchor constant:-kPadding],
        [g_progressBar.centerYAnchor constraintEqualToAnchor:progressRow.centerYAnchor constant:-2],
    ]];

    // Control bar layout
    [NSLayoutConstraint activateConstraints:@[
        [g_playBtn.centerXAnchor constraintEqualToAnchor:ctrlBar.centerXAnchor],
        [g_playBtn.centerYAnchor constraintEqualToAnchor:ctrlBar.centerYAnchor],
        [g_skipBackBtn.trailingAnchor constraintEqualToAnchor:g_playBtn.leadingAnchor constant:-12],
        [g_skipBackBtn.centerYAnchor constraintEqualToAnchor:ctrlBar.centerYAnchor],
        [g_skipFwdBtn.leadingAnchor constraintEqualToAnchor:g_playBtn.trailingAnchor constant:12],
        [g_skipFwdBtn.centerYAnchor constraintEqualToAnchor:ctrlBar.centerYAnchor],
        [g_timeLabel.leadingAnchor constraintEqualToAnchor:ctrlBar.leadingAnchor constant:kPadding],
        [g_timeLabel.centerYAnchor constraintEqualToAnchor:ctrlBar.centerYAnchor],
        [g_fileLabel.trailingAnchor constraintEqualToAnchor:ctrlBar.trailingAnchor constant:-kPadding],
        [g_fileLabel.centerYAnchor constraintEqualToAnchor:ctrlBar.centerYAnchor],
    ]];

    // Help overlay (hidden initially)
    g_helpOverlay = keyView; // keyView draws help in its drawRect
    g_helpVisible = NO;

    // ── Slider action ─────────────────────────────────────────────────
    // We use a block-based target-action via a helper category
    // For simplicity, we wire the slider to seek in the keyDown path.
    // Actually, set up a proper target-action:
    g_progressBar.target = keyView;
    g_progressBar.action = @selector(sliderChanged:);

    // Show window
    [g_window makeKeyAndOrderFront:nil];
    [g_window makeFirstResponder:keyView];
    [NSApp activateIgnoringOtherApps:YES];
}

// ── Slider handler (on KeyView) ──────────────────────────────────────
@interface KeyView (Slider)
- (void)sliderChanged:(NSSlider *)sender;
@end
@implementation KeyView (Slider)
- (void)sliderChanged:(NSSlider *)sender {
    if (!g_player || !g_player.currentItem) return;
    CMTime dur = g_player.currentItem.duration;
    if (CMTIME_IS_INDEFINITE(dur)) return;
    double total = CMTimeGetSeconds(dur);
    double frac  = sender.doubleValue / sender.maxValue;
    double secs  = frac * total;

    g_sliderInUse = YES;
    [self updateTimeLabel:secs total:total];
    [g_player seekToTime:CMTimeMakeWithSeconds(secs, 600)
         toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero
       completionHandler:^(BOOL done) {
        dispatch_async(dispatch_get_main_queue(), ^{ g_sliderInUse = NO; });
    }];
}

- (void)updateTimeLabel:(double)cur total:(double)total {
    int cm = (int)cur / 60, cs = (int)cur % 60;
    int dm = (int)total / 60, ds = (int)total % 60;
    g_timeLabel.stringValue = [NSString stringWithFormat:@"%d:%02d / %d:%02d", cm, cs, dm, ds];
}
@end

// ── Skip button actions (wired up in avplayer_init via target-action) ─
// We'll wire them up after KeyView is created. We do this via a setup
// function since KeyView only exists after avplayer_init runs.
// For now, the skip buttons use block-based actions set up after init.
static void wire_controls(void) {
    // The KeyView is g_window.contentView.subviews[0] (keyView)
    NSView *keyView = g_window.contentView.subviews[0];
    g_skipBackBtn.target = keyView;
    g_skipBackBtn.action = @selector(skipBack:);
    g_skipFwdBtn.target  = keyView;
    g_skipFwdBtn.action  = @selector(skipForward:);
    g_playBtn.target     = keyView;
    g_playBtn.action     = @selector(togglePlayPauseBtn:);
}

@interface KeyView (Buttons)
- (void)skipBack:(id)sender;
- (void)skipForward:(id)sender;
- (void)togglePlayPauseBtn:(id)sender;
@end
@implementation KeyView (Buttons)
- (void)skipBack:(id)sender { enqueue_key('c'); }
- (void)skipForward:(id)sender { enqueue_key('v'); }
- (void)togglePlayPauseBtn:(id)sender { [self togglePlayPause]; }
@end

void avplayer_pump_once(void) {
    @autoreleasepool {
        NSEvent *event;
        while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                           untilDate:[NSDate dateWithTimeIntervalSinceNow:0.005]
                                              inMode:NSDefaultRunLoopMode
                                             dequeue:YES])) {
            [NSApp sendEvent:event];
        }
    }
}

void avplayer_destroy(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (g_timeObserver && g_player) {
            [g_player removeTimeObserver:g_timeObserver];
            g_timeObserver = nil;
        }
        [g_player pause];
        g_player = nil;
        [g_window close];
    });
}

void avplayer_load(const char *path) {
    NSString *p = [NSString stringWithUTF8String:path];
    dispatch_async(dispatch_get_main_queue(), ^{
        // Clean up previous observer
        if (g_timeObserver && g_player) {
            [g_player removeTimeObserver:g_timeObserver];
            g_timeObserver = nil;
        }

        // Remove old end-of-playback observer
        [[NSNotificationCenter defaultCenter] removeObserver:nil
            name:AVPlayerItemDidPlayToEndTimeNotification object:nil];

        NSURL *url = [NSURL fileURLWithPath:p];
        AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];

        if (!g_player) {
            g_player = [AVPlayer playerWithPlayerItem:item];
            g_playerView.player = g_player;
            wire_controls();
        } else {
            [g_player replaceCurrentItemWithPlayerItem:item];
        }

        // End-of-playback notification
        [[NSNotificationCenter defaultCenter] addObserverForName:AVPlayerItemDidPlayToEndTimeNotification
            object:item queue:[NSOperationQueue mainQueue]
            usingBlock:^(NSNotification *n) {
                g_videoFinished = YES;
                KeyView *kv = (KeyView *)g_window.contentView.subviews[0];
                [kv setPlaying:NO];
            }];

        // Periodic time observer (4 Hz)
        __weak NSView *weakKV = g_window.contentView.subviews[0];
        g_timeObserver = [g_player addPeriodicTimeObserverForInterval:CMTimeMakeWithSeconds(0.25, 600)
            queue:dispatch_get_main_queue() usingBlock:^(CMTime t) {
                NSView *kv = weakKV;
                if (!kv || g_sliderInUse) return;
                double cur = CMTimeGetSeconds(t);
                double dur = 0;
                if (g_player.currentItem) {
                    CMTime d = g_player.currentItem.duration;
                    if (CMTIME_IS_VALID(d) && !CMTIME_IS_INDEFINITE(d)) dur = CMTimeGetSeconds(d);
                }
                if (dur <= 0) return;
                [(KeyView *)kv updateTimeLabel:cur total:dur];
                [g_progressBar setDoubleValue:(cur / dur) * g_progressBar.maxValue];
            }];

        [g_player play];
        KeyView *kv = (KeyView *)g_window.contentView.subviews[0];
        [kv setPlaying:YES];
        g_videoFinished = NO;
        g_videoGeneration++;

        // Reset slider
        [g_progressBar setDoubleValue:0];
        [g_timeLabel setStringValue:@"0:00 / 0:00"];
        [g_window makeFirstResponder:kv];
    });
}

void avplayer_play(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (g_player) [g_player play];
        KeyView *kv = (KeyView *)g_window.contentView.subviews[0];
        [kv setPlaying:YES];
    });
}

void avplayer_pause(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (g_player) [g_player pause];
        KeyView *kv = (KeyView *)g_window.contentView.subviews[0];
        [kv setPlaying:NO];
    });
}

int avplayer_is_playing(void) {
    return (g_player && g_player.rate > 0) ? 1 : 0;
}

double avplayer_current_time(void) {
    if (!g_player) return 0;
    return CMTimeGetSeconds(g_player.currentTime);
}

double avplayer_duration(void) {
    if (!g_player || !g_player.currentItem) return 0;
    CMTime d = g_player.currentItem.duration;
    if (CMTIME_IS_VALID(d) && !CMTIME_IS_INDEFINITE(d))
        return CMTimeGetSeconds(d);
    return 0;
}

int avplayer_did_finish(void) {
    return g_videoFinished ? 1 : 0;
}

void avplayer_seek(double seconds) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (!g_player) return;
        [g_player seekToTime:CMTimeMakeWithSeconds(seconds, 600)
             toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
    });
}

void avplayer_seek_fraction(double fraction) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (!g_player || !g_player.currentItem) return;
        CMTime d = g_player.currentItem.duration;
        if (CMTIME_IS_VALID(d) && !CMTIME_IS_INDEFINITE(d)) {
            double sec = CMTimeGetSeconds(d) * fraction;
            [g_player seekToTime:CMTimeMakeWithSeconds(sec, 600)
                 toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
        }
    });
}

void avplayer_toggle_fullscreen(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [g_window toggleFullScreen:nil];
    });
}

int avplayer_is_fullscreen(void) {
    return ([g_window styleMask] & NSWindowStyleMaskFullScreen) ? 1 : 0;
}

void avplayer_set_file_label(const char *text) {
    NSString *s = [NSString stringWithUTF8String:text];
    dispatch_async(dispatch_get_main_queue(), ^{
        g_fileLabel.stringValue = s;
    });
}
