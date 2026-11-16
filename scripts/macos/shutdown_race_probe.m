// Test-only probe: does a notification removed right before process exit
// stay withdrawn, mirroring litty_notifications_shutdown + exit?
#import <AppKit/AppKit.h>
#import <UserNotifications/UserNotifications.h>
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>
#ifndef PROBE_LOG
#error PROBE_LOG must name the private test log
#endif
static void record(NSString *text) {
    int fd = open(PROBE_LOG, O_CREAT | O_APPEND | O_WRONLY | O_NOFOLLOW, 0600);
    if (fd < 0) { perror("race probe log"); return; }
    dprintf(fd, "%d %s\n", getpid(), text.UTF8String);
    close(fd);
}
static NSString *arg_after(NSString *flag) {
    NSArray *args = NSProcessInfo.processInfo.arguments;
    NSUInteger i = [args indexOfObject:flag];
    return (i != NSNotFound && i + 1 < args.count) ? args[i + 1] : nil;
}
static void race(UNUserNotificationCenter *c, NSString *round, NSInteger n, BOOL next_turn) {
    NSMutableArray<NSString *> *ids = [NSMutableArray array];
    for (NSInteger i = 0; i < n; i++) {
        NSString *ident = [NSString stringWithFormat:@"race-%@-%ld", round, (long)i];
        [ids addObject:ident];
        UNMutableNotificationContent *content = [UNMutableNotificationContent new];
        content.title = @"Litty shutdown race test";
        content.body = ident;
        [c addNotificationRequest:[UNNotificationRequest requestWithIdentifier:ident content:content trigger:nil]
            withCompletionHandler:^(NSError *e) { record([NSString stringWithFormat:@"late-completion %@ %@", ident, e]); }];
    }
    void (^shutdown)(void) = ^{
        // Same calls, same order as litty_notifications_shutdown; no waiting.
        // --no-remove is the negative control proving the count can fail.
        if (!arg_after(@"--no-remove")) {
            record(@"removing");
            [c removePendingNotificationRequestsWithIdentifiers:ids];
            [c removeDeliveredNotificationsWithIdentifiers:ids];
        }
        record([NSString stringWithFormat:@"removed-and-exit round=%@ n=%ld next_turn=%d", round, (long)n, next_turn]);
        exit(0);
    };
    if (next_turn) dispatch_async(dispatch_get_main_queue(), shutdown); else shutdown();
}
static void count_and_clean(UNUserNotificationCenter *c) {
    [c getPendingNotificationRequestsWithCompletionHandler:^(NSArray<UNNotificationRequest *> *pending) {
        [c getDeliveredNotificationsWithCompletionHandler:^(NSArray<UNNotification *> *delivered) {
            NSMutableArray *left = [NSMutableArray array];
            for (UNNotificationRequest *r in pending) if ([r.identifier hasPrefix:@"race-"]) [left addObject:[@"pending:" stringByAppendingString:r.identifier]];
            for (UNNotification *d in delivered) if ([d.request.identifier hasPrefix:@"race-"]) [left addObject:[@"delivered:" stringByAppendingString:d.request.identifier]];
            record([NSString stringWithFormat:@"leftover count=%lu %@", (unsigned long)left.count, [left componentsJoinedByString:@","]]);
            NSMutableArray *ids = [NSMutableArray array];
            for (UNNotification *d in delivered) if ([d.request.identifier hasPrefix:@"race-"]) [ids addObject:d.request.identifier];
            for (UNNotificationRequest *r in pending) if ([r.identifier hasPrefix:@"race-"]) [ids addObject:r.identifier];
            [c removePendingNotificationRequestsWithIdentifiers:ids];
            [c removeDeliveredNotificationsWithIdentifiers:ids];
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, NSEC_PER_SEC), dispatch_get_main_queue(), ^{ exit(0); });
        }];
    }];
}
@interface RaceProbe : NSObject <NSApplicationDelegate>
@end
@implementation RaceProbe
- (void)applicationDidFinishLaunching:(NSNotification *)n {
    UNUserNotificationCenter *c = UNUserNotificationCenter.currentNotificationCenter;
    [c requestAuthorizationWithOptions:UNAuthorizationOptionAlert completionHandler:^(BOOL yes, NSError *e) {
        record([NSString stringWithFormat:@"permission %d %@", yes, e]);
        if (!yes || e) exit(3);
        dispatch_async(dispatch_get_main_queue(), ^{
            NSString *round = arg_after(@"--race");
            if (round) race(c, round, arg_after(@"--n").integerValue ?: 10, arg_after(@"--next-turn") != nil);
            else {
                // Delivery lags the exited sender by seconds; let usernoted settle.
                int64_t settle = (arg_after(@"--settle").integerValue ?: 10) * (int64_t)NSEC_PER_SEC;
                dispatch_after(dispatch_time(DISPATCH_TIME_NOW, settle), dispatch_get_main_queue(), ^{ count_and_clean(c); });
            }
        });
    }];
}
@end
int main(void) {
    @autoreleasepool {
        NSApplication *app = NSApplication.sharedApplication;
        RaceProbe *delegate = [RaceProbe new];
        app.delegate = delegate;
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        [app run];
    }
}
