#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// Synthesizes multi-touch gestures using private XCTest APIs.
/// Each finger is a dictionary with:
///   - "points": NSArray of NSArray<NSNumber *> pairs [[x, y], [x, y], ...]
///   - "duration": NSNumber (seconds)
@interface GestureSynthesizer : NSObject

+ (BOOL)synthesizeFingers:(NSArray<NSDictionary *> *)fingers
                    error:(NSError *_Nullable *_Nullable)error;

@end

NS_ASSUME_NONNULL_END
