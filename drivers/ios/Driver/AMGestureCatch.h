#import <Foundation/Foundation.h>

// Run an XCTest gesture block, converting an NSException raise into a
// returned message. XCTest signals an unaddressable element by raising
// (not via Swift errors), so without this the raise aborts the runner
// and kills the driver for every later verb. Returns nil on success.
@interface AMGestureCatch : NSObject
+ (nullable NSString *)runGesture:(void (^_Nonnull)(void))block;
@end
