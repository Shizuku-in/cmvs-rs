#import <UIKit/UIKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <AVFAudio/AVFAudio.h>
#include "cmvs_host.h"

@interface PlayerController : UIViewController <UIDocumentPickerDelegate> {
    CmvsHost *_player;
    UIImageView *_image;
    CADisplayLink *_clock;
    NSURL *_gameURL;
    CFTimeInterval _lastTime;
    uint64_t _serial;
    BOOL _failed;
}
@end

@implementation PlayerController
- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = UIColor.blackColor;
    _image = [[UIImageView alloc] initWithFrame:self.view.bounds];
    _image.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    _image.contentMode = UIViewContentModeScaleAspectFit;
    [self.view addSubview:_image];
    UIButton *open = [UIButton buttonWithType:UIButtonTypeSystem];
    [open setTitle:@"Open game folder" forState:UIControlStateNormal];
    open.frame = CGRectMake(28, 28, 170, 44);
    [open addTarget:self action:@selector(selectGame) forControlEvents:UIControlEventTouchUpInside];
    [self.view addSubview:open];
    NSNotificationCenter *notifications = NSNotificationCenter.defaultCenter;
    [notifications addObserver:self selector:@selector(pause) name:UIApplicationWillResignActiveNotification object:nil];
    [notifications addObserver:self selector:@selector(resume) name:UIApplicationDidBecomeActiveNotification object:nil];
    NSString *path = NSProcessInfo.processInfo.environment[@"CMVS_GAME_DIR"];
    if (path.length) dispatch_async(dispatch_get_main_queue(), ^{ [self openGame:[NSURL fileURLWithPath:path]]; });
}
- (void)showError:(NSString *)message {
    _failed = YES;
    [_clock invalidate]; _clock = nil;
    NSLog(@"CMVS error: %@", message);
    UIAlertController *alert = [UIAlertController alertControllerWithTitle:@"CMVS" message:message preferredStyle:UIAlertControllerStyleAlert];
    [alert addAction:[UIAlertAction actionWithTitle:@"OK" style:UIAlertActionStyleDefault handler:nil]];
    [self presentViewController:alert animated:YES completion:nil];
}
- (void)closeGame {
    [_clock invalidate]; _clock = nil;
    if (_player) {
        if (!_failed && cmvs_host_save_preferences(_player) < 0)
            NSLog(@"CMVS save settings: %s", cmvs_host_error(_player));
        cmvs_host_destroy(_player); _player = nullptr;
    }
    [_gameURL stopAccessingSecurityScopedResource]; _gameURL = nil;
}
- (void)selectGame {
    [self closeGame];
    UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc] initForOpeningContentTypes:@[UTTypeFolder] asCopy:NO];
    picker.delegate = self;
    [self presentViewController:picker animated:YES completion:nil];
}
- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    if (urls.count) [self openGame:urls.firstObject];
}
- (void)openGame:(NSURL *)url {
    [self closeGame];
    _failed = NO; _gameURL = url;
    [_gameURL startAccessingSecurityScopedResource];
    NSURL *documents = [NSFileManager.defaultManager URLsForDirectory:NSDocumentDirectory inDomains:NSUserDomainMask].firstObject;
    // Each selected game gets a stable independent namespace; never write in its folder.
    NSData *key = [url.path dataUsingEncoding:NSUTF8StringEncoding];
    uint64_t hash = 14695981039346656037ull;
    for (NSUInteger i=0; i<key.length; ++i) hash = (hash ^ ((const uint8_t *)key.bytes)[i])*1099511628211ull;
    NSString *user = [[documents.path stringByAppendingPathComponent:@"Saves"] stringByAppendingPathComponent:[NSString stringWithFormat:@"%016llx", (unsigned long long)hash]];
    NSString *font = [NSBundle.mainBundle pathForResource:@"font" ofType:@"otf"];
    NSError *error = nil;
    [AVAudioSession.sharedInstance setCategory:AVAudioSessionCategoryPlayback error:&error];
    if (!error) [AVAudioSession.sharedInstance setActive:YES error:&error];
    if (error) { [self showError:error.localizedDescription]; return; }
    _player = cmvs_host_create();
    if (cmvs_host_open(_player,url.path.UTF8String,user.UTF8String,font.UTF8String,1)<0) {
        [self showError:[NSString stringWithUTF8String:cmvs_host_error(_player)]]; return;
    }
    _serial = 0; _lastTime = 0;
    _clock = [CADisplayLink displayLinkWithTarget:self selector:@selector(step:)];
    _clock.preferredFramesPerSecond = 60;
    [_clock addToRunLoop:NSRunLoop.mainRunLoop forMode:NSRunLoopCommonModes];
    NSLog(@"CMVS opened %@ (%ux%u)", url.lastPathComponent,cmvs_host_width(_player),cmvs_host_height(_player));
}
- (void)pause { if (_player) cmvs_host_pause(_player,1); _clock.paused=YES; }
- (void)resume { if (_player) cmvs_host_pause(_player,0); _clock.paused=NO; _lastTime=0; }
- (void)step:(CADisplayLink *)clock {
    uint32_t delta = _lastTime ? (uint32_t)MIN(250.0,(clock.timestamp-_lastTime)*1000.0) : 16;
    int result = cmvs_host_tick(_player,delta);
    if (result<0) { [self showError:[NSString stringWithUTF8String:cmvs_host_error(_player)]]; return; }
    if (result==2) { [self closeGame]; return; }
    if (result!=1) return;
    _lastTime=clock.timestamp;
    uint32_t width=cmvs_host_width(_player),height=cmvs_host_height(_player);
    NSData *data=[NSData dataWithBytes:cmvs_host_pixels(_player) length:(NSUInteger)width*height*4];
    CGDataProviderRef provider=CGDataProviderCreateWithCFData((__bridge CFDataRef)data);
    CGColorSpaceRef space=CGColorSpaceCreateDeviceRGB();
    CGImageRef image=CGImageCreate(width,height,8,32,width*4,space,kCGBitmapByteOrder32Big|kCGImageAlphaLast,provider,NULL,false,kCGRenderingIntentDefault);
    _image.image=[UIImage imageWithCGImage:image];
    CGImageRelease(image); CGColorSpaceRelease(space); CGDataProviderRelease(provider);
    _serial=cmvs_host_frame_serial(_player);
    // Optional simulator diagnostics. The same tick, input and presentation path is used.
    NSDictionary *env=NSProcessInfo.processInfo.environment;
    NSUInteger limit=[env[@"CMVS_SMOKE_FRAMES"] integerValue];
    if (limit && _serial==limit) {
        NSString *output=env[@"CMVS_SMOKE_OUTPUT"];
        if (output.length) [UIImagePNGRepresentation(_image.image) writeToFile:output atomically:YES];
        NSLog(@"CMVS_SMOKE_OK frames=%llu",(unsigned long long)_serial);
        [self closeGame];
    }
}
- (void)pointer:(UITouch *)touch pressed:(BOOL)pressed {
    if (!_player || _failed) return;
    CGPoint point=[touch locationInView:_image];
    CGFloat w=cmvs_host_width(_player),h=cmvs_host_height(_player);
    CGFloat scale=MIN(_image.bounds.size.width/w,_image.bounds.size.height/h);
    CGFloat x=(point.x-(_image.bounds.size.width-w*scale)/2)/scale;
    CGFloat y=(point.y-(_image.bounds.size.height-h*scale)/2)/scale;
    cmvs_host_pointer(_player,(int32_t)x,(int32_t)y);
    cmvs_host_key(_player,1,pressed);
}
- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { [self pointer:touches.anyObject pressed:YES]; }
- (void)touchesMoved:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { [self pointer:touches.anyObject pressed:YES]; }
- (void)touchesEnded:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { [self pointer:touches.anyObject pressed:NO]; }
- (void)touchesCancelled:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { if (_player) cmvs_host_key(_player,1,0); }
- (void)dealloc { [self closeGame]; [NSNotificationCenter.defaultCenter removeObserver:self]; }
@end

@interface AppDelegate : UIResponder <UIApplicationDelegate>
@property(strong,nonatomic) UIWindow *window;
@end
@implementation AppDelegate
- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)options {
    self.window=[[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];
    self.window.rootViewController=[PlayerController new];
    [self.window makeKeyAndVisible]; return YES;
}
@end
int main(int argc,char **argv) { @autoreleasepool {return UIApplicationMain(argc,argv,nil,NSStringFromClass(AppDelegate.class));} }
