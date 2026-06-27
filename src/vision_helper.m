// Objective-C helper for macOS Vision framework OCR
// Compiled at build time by build.rs, linked into the binary.

#import <Vision/Vision.h>
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>
#import <Cocoa/Cocoa.h>

// --- Global result storage ---
typedef struct {
    char* text;
    double confidence;
    double x, y, w, h;       // bounding rect in pixel coords
} OCRResult;

static OCRResult*  g_results        = NULL;
static int         g_result_count   = 0;
static int         g_result_capacity = 0;

static void clear_results(void) {
    if (g_results) {
        for (int i = 0; i < g_result_count; i++) {
            free(g_results[i].text);
        }
        free(g_results);
        g_results = NULL;
    }
    g_result_count = 0;
    g_result_capacity = 0;
}

static void add_result(const char* text, double confidence,
                       double x, double y, double w, double h)
{
    if (g_result_count >= g_result_capacity) {
        g_result_capacity = (g_result_capacity == 0) ? 16 : g_result_capacity * 2;
        g_results = realloc(g_results, (size_t)g_result_capacity * sizeof(OCRResult));
    }
    OCRResult* r = &g_results[g_result_count++];
    r->text       = strdup(text);
    r->confidence = confidence;
    r->x = x; r->y = y; r->w = w; r->h = h;
}

// --- Main entry point ---
// `data` = JPEG bytes, `len` = byte count.
// Returns number of results (>=0) or negative error code.
int vision_perform_ocr(const unsigned char* data, size_t len)
{
    @autoreleasepool {
        clear_results();

        NSData* nsdata = [NSData dataWithBytes:data length:len];
        CGImageSourceRef src = CGImageSourceCreateWithData((__bridge CFDataRef)nsdata, NULL);
        if (!src) return -1;

        CGImageRef cgImage = CGImageSourceCreateImageAtIndex(src, 0, NULL);
        CFRelease(src);
        if (!cgImage) return -2;

        VNImageRequestHandler* handler = [[VNImageRequestHandler alloc]
            initWithCGImage:cgImage options:@{}];

        VNRecognizeTextRequest* req = [[VNRecognizeTextRequest alloc] init];
        req.recognitionLevel = VNRequestTextRecognitionLevelAccurate;

        NSError* error = nil;
        BOOL ok = [handler performRequests:@[req] error:&error];
        if (!ok) {
            NSLog(@"Vision OCR error: %@", error.localizedDescription);
            CGImageRelease(cgImage);
            return -3;
        }

        CGFloat imgW = CGImageGetWidth(cgImage);
        CGFloat imgH = CGImageGetHeight(cgImage);

        for (VNRecognizedTextObservation* obs in req.results) {
            NSArray<VNRecognizedText*>* top = [obs topCandidates:1];
            if (top.count == 0) continue;

            VNRecognizedText* rt = top[0];
            CGRect bb = obs.boundingBox;

            double pixelX = bb.origin.x * imgW;
            double pixelY = (1.0 - bb.origin.y - bb.size.height) * imgH;
            double pixelW = bb.size.width  * imgW;
            double pixelH = bb.size.height * imgH;

            add_result([rt.string UTF8String], rt.confidence,
                       pixelX, pixelY, pixelW, pixelH);
        }

        CGImageRelease(cgImage);
        return g_result_count;
    }
}

// --- C accessors (called from Rust) ---
int vision_result_count(void) { return g_result_count; }

const char* vision_result_text(int idx) {
    if (idx < 0 || idx >= g_result_count) return NULL;
    return g_results[idx].text;
}

double vision_result_confidence(int idx) {
    if (idx < 0 || idx >= g_result_count) return 0.0;
    return g_results[idx].confidence;
}

// Returns a pointer to a 4-double array [x, y, w, h]
const double* vision_result_rect(int idx) {
    if (idx < 0 || idx >= g_result_count) return NULL;
    OCRResult* r = &g_results[idx];
    // Store in a static buffer — only 4 doubles, safe.
    static double rect[4];
    rect[0] = r->x; rect[1] = r->y; rect[2] = r->w; rect[3] = r->h;
    return rect;
}
