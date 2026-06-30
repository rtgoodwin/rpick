// Native macOS window helpers for OpenCV HighGUI windows
// Mirrors gopick's window_aspect_darwin.go helpers.

#import <Cocoa/Cocoa.h>

static NSWindow *rpick_find_window(const char *title) {
    NSString *windowTitle = [NSString stringWithUTF8String:title];
    for (NSWindow *window in [NSApp windows]) {
        if ([[window title] containsString:windowTitle]) {
            return window;
        }
    }
    return nil;
}

int rpick_set_window_aspect_ratio(const char *title, double width, double height) {
    @autoreleasepool {
        NSWindow *window = rpick_find_window(title);
        if (window == nil) return 0;
        [window setContentAspectRatio:NSMakeSize(width, height)];
        return 1;
    }
}

int rpick_clear_window_aspect_ratio(const char *title) {
    @autoreleasepool {
        NSWindow *window = rpick_find_window(title);
        if (window == nil) return 0;
        [window setContentAspectRatio:NSMakeSize(0, 0)];
        return 1;
    }
}

int rpick_get_window_content_size(const char *title, int *out_width, int *out_height) {
    @autoreleasepool {
        NSWindow *window = rpick_find_window(title);
        if (window == nil) return 0;
        NSRect contentRect = [[window contentView] frame];
        *out_width = (int)contentRect.size.width;
        *out_height = (int)contentRect.size.height;
        return 1;
    }
}

int rpick_set_window_standard_decorations(const char *title) {
    @autoreleasepool {
        NSWindow *window = rpick_find_window(title);
        if (window == nil) return 0;
        NSWindowStyleMask style = NSWindowStyleMaskTitled |
                                  NSWindowStyleMaskClosable |
                                  NSWindowStyleMaskMiniaturizable |
                                  NSWindowStyleMaskResizable;
        [window setStyleMask:style];
        [window makeKeyAndOrderFront:nil];
        return 1;
    }
}

int rpick_toggle_window_fullscreen(const char *title) {
    @autoreleasepool {
        NSWindow *window = rpick_find_window(title);
        if (window == nil) return 0;
        [window toggleFullScreen:nil];
        return 1;
    }
}

int rpick_is_window_fullscreen(const char *title) {
    @autoreleasepool {
        NSWindow *window = rpick_find_window(title);
        if (window == nil) return 0;
        NSWindowStyleMask style = [window styleMask];
        return (style & NSWindowStyleMaskFullScreen) ? 1 : 0;
    }
}
