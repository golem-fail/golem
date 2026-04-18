#import "GestureSynthesizer.h"
#import <XCTest/XCTest.h>

// Private XCTest class declarations (in XCUIAutomation.framework).
// Verified via nm against Xcode 16.4.

@interface XCPointerEventPath : NSObject
- (instancetype)initForTouchAtPoint:(CGPoint)point offset:(NSTimeInterval)offset;
- (void)moveToPoint:(CGPoint)point atOffset:(NSTimeInterval)offset;
- (void)liftUpAtOffset:(NSTimeInterval)offset;
@end

@interface XCSynthesizedEventRecord : NSObject
- (instancetype)initWithName:(NSString *)name interfaceOrientation:(UIInterfaceOrientation)orientation;
- (void)addPointerEventPath:(XCPointerEventPath *)path;
// Direct synchronous dispatch — no completion block, no eventSynthesizer needed.
- (BOOL)synthesizeWithError:(NSError **)error;
@end

@implementation GestureSynthesizer

+ (BOOL)synthesizeFingers:(NSArray<NSDictionary *> *)fingers
                    error:(NSError *__autoreleasing *)error {
    if (fingers.count == 0) {
        if (error) {
            *error = [NSError errorWithDomain:@"GestureSynthesizer"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey: @"No fingers provided"}];
        }
        return NO;
    }

    @try {
        XCSynthesizedEventRecord *eventRecord =
            [[XCSynthesizedEventRecord alloc] initWithName:@"golem-gesture"
                                      interfaceOrientation:UIInterfaceOrientationPortrait];

        for (NSUInteger f = 0; f < fingers.count; f++) {
            NSDictionary *finger = fingers[f];
            NSArray *points = finger[@"points"];
            NSNumber *durationNum = finger[@"duration"];
            NSTimeInterval duration = durationNum ? durationNum.doubleValue : 0.3;

            if (points.count < 2) {
                if (error) {
                    *error = [NSError errorWithDomain:@"GestureSynthesizer"
                                                 code:2
                                             userInfo:@{NSLocalizedDescriptionKey: @"Each finger needs >= 2 points"}];
                }
                return NO;
            }

            NSArray *firstPt = points[0];
            CGPoint firstPoint = CGPointMake([firstPt[0] doubleValue], [firstPt[1] doubleValue]);
            XCPointerEventPath *path = [[XCPointerEventPath alloc] initForTouchAtPoint:firstPoint offset:0];

            for (NSUInteger i = 1; i < points.count; i++) {
                NSArray *ptArr = points[i];
                CGPoint pt = CGPointMake([ptArr[0] doubleValue], [ptArr[1] doubleValue]);
                NSTimeInterval offset = duration * (double)i / (double)(points.count - 1);
                [path moveToPoint:pt atOffset:offset];
            }

            [path liftUpAtOffset:duration + 0.01];
            [eventRecord addPointerEventPath:path];
        }

        NSError *synthError = nil;
        BOOL result = [eventRecord synthesizeWithError:&synthError];

        if (!result && synthError) {
            if (error) {
                *error = synthError;
            }
            return NO;
        }
        return result;

    } @catch (NSException *exception) {
        NSLog(@"[golem] Exception: %@ - %@", exception.name, exception.reason);
        if (error) {
            *error = [NSError errorWithDomain:@"GestureSynthesizer"
                                         code:99
                                     userInfo:@{NSLocalizedDescriptionKey:
                [NSString stringWithFormat:@"Exception: %@ - %@", exception.name, exception.reason]}];
        }
        return NO;
    }
}

@end
