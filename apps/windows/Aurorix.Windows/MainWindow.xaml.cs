using System;
using System.Globalization;
using Aurorix.Windows.Home;
using Aurorix.Windows.Settings;
using Aurorix.Windows.Themes;
using Microsoft.UI;
using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Windows.Foundation;
using Windows.Graphics;
using Windows.Storage;
using Windows.UI;
using Windows.UI.ViewManagement;
using WinRT.Interop;

namespace Aurorix.Windows;

public sealed partial class MainWindow : Window
{
    private const int MinWidth = 960; // 窗口允许的最小客户区宽度。
    private const int MinHeight = 640; // 窗口允许的最小客户区高度。
    private const int DefaultWidth = 1280; // 没有历史尺寸时使用的初始宽度。
    private const int DefaultHeight = 800; // 没有历史尺寸时使用的初始高度。
    private const double DefaultAcrylicTransparency = 0.4; // 自定义 Acrylic 的默认通透度。
    private const double DefaultAcrylicVividness = 0.6; // 自定义 Acrylic 的默认鲜艳度（用户侧方向）。
    private const double AcrylicVividnessLuminosityNudge = 0.01; // 仅用于触发鲜艳度变化的最大亮度微扰。
    private const string WidthKey = "Aurorix.Window.Width"; // LocalSettings 中保存窗口宽度的键。
    private const string HeightKey = "Aurorix.Window.Height"; // LocalSettings 中保存窗口高度的键。
    private const string MaterialKey = "Aurorix.Theme.Material"; // LocalSettings 中保存主题预设的键。
    private const string VariantKey = "Aurorix.Theme.Variant"; // LocalSettings 中保存明暗模式的键。
    private const string AcrylicTransparencyKey = "Aurorix.Theme.CustomAcrylic.Transparency"; // 保存通透度滑块值的键。
    private const string AcrylicVividnessKey = "Aurorix.Theme.CustomAcrylic.Vividness"; // 保存鲜艳度滑块值的键。

    private readonly ApplicationDataContainer? _settings; // 可能不可用的 LocalSettings；所有读写都按可空处理。
    private readonly AccessibilitySettings _accessibilitySettings = new(); // 提供高对比度状态和变化通知。
    private AppWindow? _appWindow; // WinUI Window 与窗口管理 API 之间的桥接对象。
    private SettingsPage? _settingsPage; // 当前设置页实例，用于读取用户选择。
    private ThemeMaterial _material; // 用户选择的主题预设；高对比度时不会直接使用它。
    private ThemeSystemVariant _variant; // 用户选择的明暗模式。
    private double _acrylicTransparency; // 用户侧通透度值，范围固定为 0 到 1。
    private double _acrylicVividness; // 用户侧鲜艳度值，范围固定为 0 到 1。
    private ConfigurableDesktopAcrylicBackdrop? _configurableAcrylicBackdrop; // 三种 Acrylic 预设共用的 controller-backed backdrop。
    private ThemeMaterial? _appliedMaterial; // 最近一次成功应用的主题，用于跳过无意义的重复切换。
    private ThemeSystemVariant? _appliedVariant; // 最近一次成功应用的明暗模式，用于避免重复刷新资源。
    private bool _themeApplyQueued; // 防止滑块连续事件排入多个主题刷新任务。
    private DispatcherQueueTimer? _settingsSaveTimer; // 合并连续设置变更后再写入 LocalSettings。
    private InputNonClientPointerSource? _nonClientPointerSource; // 只为标题栏间隔像素建立原生输入边界。
    private PlaybackMode _playbackMode = PlaybackMode.ListRepeat; // 播放模式按钮当前显示的状态。
    private bool _isPlaying; // 播放/暂停按钮当前显示的状态。
    private bool _isFavorite; // 喜欢按钮当前显示的状态。

    public MainWindow()
    {
        InitializeComponent(); // 先构造 XAML 命名元素，后续主题和事件逻辑都依赖它们。
        ExtendsContentIntoTitleBar = true; // 让自定义内容延伸到系统标题栏区域。
        SetTitleBar(AppTitleBar); // 指定 XAML 中可拖动窗口的标题栏区域。
        AppTitleBar.Loaded += TitleBar_Loaded; // 首次布局完成后才能取得间隔元素的实际矩形。
        AppTitleBar.SizeChanged += TitleBar_SizeChanged; // 窗口缩放或 DPI 变化后重算间隔矩形。
        FloatingNavigationView.ItemInvoked += Navigation_ItemInvoked; // 导航选择统一交给窗口路由。

        _settings = TryGetSettings(); // LocalSettings 不可用时仍允许窗口继续启动。
        _material = ReadMaterial(); // 读取主题预设；未知值回退到云母。
        _variant = ReadVariant(); // 读取明暗模式；没有保存值时根据系统背景推断。
        _acrylicTransparency = ReadDouble(AcrylicTransparencyKey, DefaultAcrylicTransparency); // 恢复通透度滑块。
        _acrylicVividness = ReadDouble(AcrylicVividnessKey, DefaultAcrylicVividness); // 恢复鲜艳度滑块。
        PrepareInitialTheme(); // 先设置根元素主题，避免首次加载时短暂使用错误资源。
        TrySubscribeToAccessibilityChanges(); // 高对比度变化需要重新选择主题。
        ConfigureWindow(); // 建立 AppWindow 桥接并恢复窗口尺寸。
        ApplyTheme(); // 应用明暗模式、标题栏和系统 backdrop。
        ShowHomePage(); // 首次显示主页内容。
        Closed += MainWindow_Closed; // 关闭时保存设置并解除系统事件订阅。
    }

    private void PrepareInitialTheme() =>
        RootGrid.RequestedTheme = _variant == ThemeSystemVariant.Dark ? ElementTheme.Dark : ElementTheme.Light; // 在首次渲染前确定根元素的资源主题。

    private void TrySubscribeToAccessibilityChanges()
    {
        try
        {
            _accessibilitySettings.HighContrastChanged += AccessibilitySettings_HighContrastChanged; // 系统辅助功能变化时重新应用主题。
        }
        catch (Exception)
        {
        }
    }

    private void AccessibilitySettings_HighContrastChanged(AccessibilitySettings sender, object args)
    {
        if (RootGrid.DispatcherQueue.HasThreadAccess) // 主题对象只能在 UI 线程安全地更新。
        {
            ApplyTheme();
        }
        else
        {
            _ = RootGrid.DispatcherQueue.TryEnqueue(ApplyTheme); // 非 UI 线程事件转回窗口线程。
        }
    }

    private void ApplyTheme()
    {
        try
        {
            if (_appliedVariant != _variant) // 明暗模式不变时不重复刷新整棵资源树。
            {
                var requestedTheme = _variant == ThemeSystemVariant.Dark
                    ? ElementTheme.Dark
                    : ElementTheme.Light;
                RootGrid.RequestedTheme = requestedTheme; // 根元素负责向普通控件传播明暗资源。

                var titleInk = _variant == ThemeSystemVariant.Dark ? Colors.White : Colors.Black; // 标题栏图标在深浅色下使用相反的前景色。
                FloatingNavigationView.RequestedTheme = requestedTheme; // 导航控件本身也需要同步主题。
                FloatingNavigationView.ApplyThemeResources(
                    new SolidColorBrush(Colors.Transparent),
                    new SolidColorBrush(titleInk)); // 导航资源只覆盖背景/前景，不使用实验 token。
                ApplyTitleBarColors(
                    titleInk,
                    Colors.Transparent); // 标题栏按钮的 hover/pressed 色由窗口 API 统一设置。
                _appliedVariant = _variant; // 记录成功应用的模式，供下一次刷新比较。
            }

            var material = _accessibilitySettings.HighContrast ? ThemeMaterial.Solid : _material; // 高对比度强制使用不透明纯色。
            ApplyMaterial(material); // 主题变化和 Acrylic 参数变化都从这里进入。
        }
        catch (Exception)
        {
            // 主题变化可能与窗口销毁交错；这里只隔离托管 XAML 异常，不能把它当作原生访问冲突的保证。
        }
    }

    private void ApplyMaterial(ThemeMaterial material)
    {
        try
        {
            switch (material)
            {
                case ThemeMaterial.Mica:
                case ThemeMaterial.MicaAlt:
                    if (_appliedMaterial != material) // 云母是独立主题，只有主题真正改变时才替换 backdrop。
                    {
                        SystemBackdrop = new MicaBackdrop
                        {
                            Kind = material == ThemeMaterial.MicaAlt ? MicaKind.BaseAlt : MicaKind.Base,
                        }; // Mica 与 Mica Alt 共享实现，但分别选择 Windows 的 Base/BaseAlt 变体。
                    }
                    _configurableAcrylicBackdrop = null; // 离开 Acrylic 家族后丢弃共享 controller 引用。
                    break;
                case ThemeMaterial.Acrylic:
                    // 常规、通透和自定义 Acrylic 共享同一个 controller，只通过默认参数区分。
                    ApplyConfigurableAcrylic(
                        DesktopAcrylicKind.Base,
                        GetAcrylicTintColor(),
                        tintOpacity: 0.6,
                        luminosityOpacity: 0.6);
                    break;
                case ThemeMaterial.TransparentAcrylic:
                    // 通透预设仍是可配置 Acrylic，只采用 Thin 和较低的默认 opacity。
                    ApplyConfigurableAcrylic(
                        DesktopAcrylicKind.Thin,
                        GetAcrylicTintColor(),
                        tintOpacity: 0.16,
                        luminosityOpacity: 0.18);
                    break;
                case ThemeMaterial.CustomAcrylic:
                    // 自定义预设把两个设置滑块转换成 controller 参数。
                    ApplyConfigurableAcrylic(
                        DesktopAcrylicKind.Base,
                        GetAcrylicTintColor(),
                        tintOpacity: GetCustomAcrylicVividness(),
                        luminosityOpacity: GetCustomAcrylicLuminosityOpacity());
                    break;
                case ThemeMaterial.None:
                    if (_appliedMaterial != material) // None 明确表示不使用系统 backdrop。
                    {
                        SystemBackdrop = null;
                    }
                    _configurableAcrylicBackdrop = null; // 没有 backdrop 时不保留 Acrylic controller。
                    break;
                default:
                    if (_appliedMaterial != material) // Solid/兼容值不应反复触发 SystemBackdrop 重置。
                    {
                        SystemBackdrop = null; // 清除系统主题，下面的 ThemeSurface 提供纯色背景。
                    }
                    _configurableAcrylicBackdrop = null; // 非 Acrylic 主题不保留 controller。
                    break;
            }

            _appliedMaterial = material; // 只有 switch 完成后才把本次主题视为已应用。
        }
        catch (Exception)
        {
            try
            {
                SystemBackdrop = null; // 失败时尽力退回无 backdrop 状态。
            }
            catch (Exception)
            {
            }
            _configurableAcrylicBackdrop = null; // 清掉可能已失效的 controller 引用。
            _appliedMaterial = null; // 下次刷新必须重新建立主题状态。
        }

        if (material == ThemeMaterial.None)
        {
            RootGrid.Background = null; // None 连根容器都不绘制背景，只保留子控件自己的内容。
            ThemeSurface.Background = null; // 不绘制主题覆盖层，避免留下任何主题或纯色填充。
            ThemeSurface.Opacity = 0; // 即使未来资源提供背景，也不让 None 合成出窗口底色。
        }
        else
        {
            RootGrid.Background ??= new SolidColorBrush(Colors.Transparent); // 离开 None 后恢复不着色的根背景。
            ThemeSurface.Background = material == ThemeMaterial.Solid // Solid 模式用内容层填充可预测的背景色。
                ? new SolidColorBrush(_variant == ThemeSystemVariant.Dark
                    ? Color.FromArgb(255, 32, 32, 32)
                    : Color.FromArgb(255, 243, 243, 243))
                : null;
            ThemeSurface.Opacity = 1; // backdrop 的透明度由 controller 管理，不在内容层再次叠加。
        }
    }

    private void ApplyConfigurableAcrylic(
        DesktopAcrylicKind kind,
        Color tintColor,
        double tintOpacity,
        double luminosityOpacity)
    {
        var fallbackColor = Color.FromArgb( // 不支持系统 Acrylic 时使用的纯色回退。
            255,
            tintColor.R,
            tintColor.G,
            tintColor.B);
        if (_configurableAcrylicBackdrop is not null &&
            IsAcrylicMaterial(_appliedMaterial)) // Acrylic 预设切换只更新同一个 controller。
        {
            _configurableAcrylicBackdrop.Update(
                kind,
                tintColor,
                tintOpacity,
                luminosityOpacity,
                fallbackColor);
        }
        else
        {
            _configurableAcrylicBackdrop = new ConfigurableDesktopAcrylicBackdrop( // 首次进入 Acrylic 家族时创建 controller-backed backdrop。
                kind,
                tintColor,
                tintOpacity,
                luminosityOpacity,
                fallbackColor);
            SystemBackdrop = _configurableAcrylicBackdrop; // 赋给窗口后，WinUI 才会触发 target connected 生命周期。
        }
    }

    private static bool IsAcrylicMaterial(ThemeMaterial? material) =>
        material is ThemeMaterial.Acrylic
            or ThemeMaterial.TransparentAcrylic
            or ThemeMaterial.CustomAcrylic; // 枚举仍区分预设，但运行时实现统一为 Acrylic controller。

    private double GetCustomAcrylicLuminosityOpacity()
    {
        var transparency = Math.Clamp(_acrylicTransparency, 0, 1); // 通透度是这个参数的主控输入。
        var vividness = GetCustomAcrylicVividness(); // 先换算成实际传给 Windows 的鲜艳度方向。

        // Desktop Acrylic can visually ignore a TintOpacity-only update on
        // some Windows compositions. Keep transparency as the primary input,
        // while adding a bounded nudge tied to vividness so that slider moves
        // reliably produce a controller update without swapping meanings.
        var vividnessNudge = (vividness - 0.5) * 2 * AcrylicVividnessLuminosityNudge; // 只做 ±0.01 的微扰，确保鲜艳度拖动能刷新组合效果。
        return Math.Clamp(1 - transparency + vividnessNudge, 0, 1); // Windows 使用的是 luminosity opacity，而 UI 暴露的是通透度。
    }

    private double GetCustomAcrylicVividness() =>
        1 - Math.Clamp(_acrylicVividness, 0, 1); // 当前 Windows 合成方向与用户直觉相反，所以这里反转滑块值。

    private Color GetAcrylicTintColor() =>
        _variant == ThemeSystemVariant.Dark
            ? Color.FromArgb(255, 20, 28, 31)
            : Color.FromArgb(255, 232, 244, 241); // 明暗模式只改变色调，Acrylic 参数仍由预设/滑块决定。

    private void ApplyTitleBarColors(Color foreground, Color background)
    {
        if (_appWindow is null)
        {
            return;
        }

        try
        {
            var titleBar = _appWindow.TitleBar; // AppWindowTitleBar 才能控制系统标题栏按钮状态。
            var transparent = Color.FromArgb(0, background.R, background.G, background.B); // 标题栏底色保持透明，让 backdrop 透出。
            var ink = Color.FromArgb(255, foreground.R, foreground.G, foreground.B); // 规范化前景色的 alpha，避免传入半透明文字色。
            titleBar.BackgroundColor = transparent; // 正常标题栏背景。
            titleBar.ForegroundColor = ink; // 正常标题栏前景。
            titleBar.ButtonBackgroundColor = transparent; // 系统按钮的默认背景。
            titleBar.ButtonForegroundColor = ink; // 系统按钮的默认图标色。
            titleBar.ButtonHoverBackgroundColor = Color.FromArgb(24, ink.R, ink.G, ink.B); // hover 使用前景色的低 alpha 叠加。
            titleBar.ButtonHoverForegroundColor = ink; // hover 时保持图标可读性。
            titleBar.ButtonPressedBackgroundColor = Color.FromArgb(48, ink.R, ink.G, ink.B); // pressed 比 hover 更明显。
            titleBar.ButtonPressedForegroundColor = ink; // pressed 时不改变图标色。
            titleBar.InactiveBackgroundColor = transparent; // 窗口失焦时仍不盖住 backdrop。
            titleBar.InactiveForegroundColor = ink; // 窗口失焦时保留相同的图标对比度。
        }
        catch (Exception)
        {
        }
    }

    private void TitleBar_Loaded(object sender, RoutedEventArgs args) =>
        UpdateTitleBarHoverBoundary();

    private void TitleBar_SizeChanged(object sender, SizeChangedEventArgs args) =>
        UpdateTitleBarHoverBoundary();

    private void UpdateTitleBarHoverBoundary()
    {
        if (_appWindow is null || AppTitleBar.XamlRoot is null)
        {
            return;
        }

        try
        {
            _nonClientPointerSource ??= InputNonClientPointerSource.GetForWindowId(_appWindow.Id);
            var scale = AppTitleBar.XamlRoot.RasterizationScale;
            var bounds = TitleBarNativeHoverGap.TransformToVisual(null).TransformBounds(
                new Rect(0, 0, TitleBarNativeHoverGap.ActualWidth, TitleBarNativeHoverGap.ActualHeight));
            var left = (int)Math.Floor(bounds.X * scale);
            var top = (int)Math.Floor(bounds.Y * scale);
            var right = (int)Math.Ceiling((bounds.X + bounds.Width) * scale);
            var bottom = (int)Math.Ceiling((bounds.Y + bounds.Height) * scale);

            // The visual one-pixel gap is still inside SetTitleBar's non-client
            // area. Mark only this boundary as passthrough so native caption
            // hover receives a real leave before the app-owned button begins.
            _nonClientPointerSource.SetRegionRects(
                NonClientRegionKind.Passthrough,
                new[] { new RectInt32(left, top, right - left, bottom - top) });
        }
        catch (Exception)
        {
            // Layout can race window teardown; the next title-bar event retries.
        }
    }

    private void Navigation_ItemInvoked(NavigationView sender, NavigationViewItemInvokedEventArgs args)
    {
        switch (args.InvokedItemContainer?.Tag as string) // Tag 是 XAML 导航项与页面路由之间的稳定契约。
        {
            case "home": ShowHomePage(); break; // 主页复用已有页面实例。
            case "settings": ShowSettingsPage(); break; // 设置页负责把选择回调给窗口。
            default: ContentFrame.Content = null; break; // 未实现的导航项暂不显示内容。
        }
    }

    private void ShowHomePage()
    {
        if (ContentFrame.Content is not HomePage) // 避免重复导航导致页面重新加载。
        {
            ContentFrame.Navigate(typeof(HomePage), null, new CommonNavigationTransitionInfo()); // 页面导航由 Frame 统一管理。
        }
    }

    private void ShowSettingsPage()
    {
        if (ContentFrame.Content is not SettingsPage) // 只有首次进入设置页时创建并订阅事件。
        {
            ContentFrame.Navigate(typeof(SettingsPage), null, new CommonNavigationTransitionInfo());
            _settingsPage = ContentFrame.Content as SettingsPage;
            if (_settingsPage is not null)
            {
                _settingsPage.SelectionChanged += Settings_SelectionChanged; // 设置页只报告变化，真正应用由窗口负责。
            }
        }

        _settingsPage?.SetSelection(_material, _variant, _acrylicTransparency, _acrylicVividness); // 每次进入都把窗口状态同步回控件。
    }

    private void Settings_SelectionChanged(object? sender, EventArgs args)
    {
        if (_settingsPage is null)
        {
            return;
        }

        _material = _settingsPage.SelectedMaterial; // 读取设置页当前主题预设。
        _variant = _settingsPage.SelectedVariant; // 读取设置页当前明暗模式。
        _acrylicTransparency = _settingsPage.CustomAcrylicTransparency; // 读取通透度滑块。
        _acrylicVividness = _settingsPage.CustomAcrylicVividness; // 读取鲜艳度滑块。
        QueueSaveThemeSettings(); // 写盘延迟合并，避免每个滑块像素都同步写入。
        QueueThemeApply(); // 主题应用也合并到 UI 队列，避免连续重建。
    }

    private void QueueThemeApply()
    {
        if (_themeApplyQueued) // 已有刷新任务时，不再重复排队。
        {
            return;
        }

        try
        {
            _themeApplyQueued = true; // 先占位，再提交任务，防止连续 ValueChanged 竞相入队。
            if (!RootGrid.DispatcherQueue.TryEnqueue(
                    DispatcherQueuePriority.Low,
                    () =>
                    {
                        _themeApplyQueued = false; // 允许下一批设置变化创建新的刷新任务。
                        ApplyTheme(); // 在 UI 线程应用控件主题和系统 backdrop。
                    }))
            {
                _themeApplyQueued = false; // DispatcherQueue 已关闭时恢复标记，避免永久阻塞后续尝试。
            }
        }
        catch (Exception)
        {
            _themeApplyQueued = false; // 窗口销毁期间队列可能抛异常，不能留下错误的“已排队”状态。
        }
    }

    private void QueueSaveThemeSettings()
    {
        if (_settings is null) // 某些未打包/受限环境没有可用 LocalSettings。
        {
            return;
        }

        try
        {
            _settingsSaveTimer ??= CreateSettingsSaveTimer(); // 首次变更时才创建防抖计时器。
            _settingsSaveTimer.Stop(); // 新变更到来时重新开始 180ms 窗口。
            _settingsSaveTimer.Start(); // 连续拖动结束后再执行一次写盘。
        }
        catch (Exception)
        {
        }
        _nonClientPointerSource = null; // 窗口销毁后不再保留间隔像素的原生输入桥接对象。
    }

    private DispatcherQueueTimer CreateSettingsSaveTimer()
    {
        var timer = RootGrid.DispatcherQueue.CreateTimer(); // 计时器绑定窗口 Dispatcher，Tick 会回到 UI 线程。
        timer.Interval = TimeSpan.FromMilliseconds(180); // 合并滑块快速变化，减少 LocalSettings 写入。
        timer.IsRepeating = false; // 每轮连续变化只保存一次。
        timer.Tick += SettingsSaveTimer_Tick; // 到期后写入当前完整主题状态。
        return timer;
    }

    private void SettingsSaveTimer_Tick(DispatcherQueueTimer sender, object args)
    {
        sender.Stop(); // 单次计时器触发后立即停止，等待下一次变更重新启动。
        SaveThemeSettings(); // 写入窗口关闭前也会再次保存，避免最后一轮丢失。
    }

    private void ConfigureWindow()
    {
        var handle = WindowNative.GetWindowHandle(this); // 取得 WinUI Window 对应的原生 HWND。
        var id = Microsoft.UI.Win32Interop.GetWindowIdFromWindow(handle); // 将 HWND 转成 AppWindow 所需的 WindowId。
        _appWindow = AppWindow.GetFromWindowId(id); // 从 WindowId 获取窗口管理对象。
        if (_appWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = MinWidth; // 设置用户拖动窗口时的最小宽度。
            presenter.PreferredMinimumHeight = MinHeight; // 设置用户拖动窗口时的最小高度。
        }
        _appWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall; // 与自定义标题栏布局保持一致。
        var area = DisplayArea.GetFromWindowId(id, DisplayAreaFallback.Primary).WorkArea; // 用主显示器工作区限制恢复尺寸。
        _appWindow.Resize(new SizeInt32(
            Clamp(ReadInt(WidthKey) ?? DefaultWidth, MinWidth, area.Width), // 恢复宽度并限制在显示器范围内。
            Clamp(ReadInt(HeightKey) ?? DefaultHeight, MinHeight, area.Height))); // 恢复高度并限制在显示器范围内。
    }

    private void MainWindow_Closed(object sender, WindowEventArgs args)
    {
        _settingsSaveTimer?.Stop(); // 关闭时取消待触发的防抖 Tick。
        SaveThemeSettings(); // 先保存最终主题值，确保最后一次拖动不丢失。
        if (_appWindow?.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Restored }) // 最大化/最小化时不覆盖上次正常尺寸。
        {
            WriteInt(WidthKey, _appWindow.Size.Width); // 保存关闭前的正常宽度。
            WriteInt(HeightKey, _appWindow.Size.Height); // 保存关闭前的正常高度。
        }
        try
        {
            _accessibilitySettings.HighContrastChanged -= AccessibilitySettings_HighContrastChanged; // 解除订阅，避免窗口销毁后仍收到回调。
        }
        catch (Exception)
        {
        }
    }

    private void PlaybackModeButton_Click(object sender, RoutedEventArgs args)
    {
        _playbackMode = _playbackMode switch // 按列表循环、随机和单曲顺序循环播放模式。
        {
            PlaybackMode.ListRepeat => PlaybackMode.Shuffle,
            PlaybackMode.Shuffle => PlaybackMode.SingleTrack,
            _ => PlaybackMode.ListRepeat,
        };
        var (symbol, label) = _playbackMode switch
        {
            PlaybackMode.Shuffle => (Symbol.Shuffle, "随机播放"),
            PlaybackMode.SingleTrack => (Symbol.RepeatOne, "单曲播放"),
            _ => (Symbol.RepeatAll, "列表循环"),
        };
        PlaybackModeIcon.Symbol = symbol; // 图标反映当前播放模式。
        ToolTipService.SetToolTip(PlaybackModeButton, label); // tooltip 同步提供无障碍/悬停说明。
    }

    private void PreviousTrackButton_Click(object sender, RoutedEventArgs args) { }
    private void CurrentTrackButton_Click(object sender, RoutedEventArgs args) { }

    private void PlayPauseButton_Click(object sender, RoutedEventArgs args)
    {
        _isPlaying = !_isPlaying; // 当前只切换界面状态，实际播放引擎接入后在这里转发命令。
        PlayPauseIcon.Symbol = _isPlaying ? Symbol.Pause : Symbol.Play; // 播放时显示暂停，暂停时显示播放。
        ToolTipService.SetToolTip(PlayPauseButton, _isPlaying ? "暂停" : "播放"); // tooltip 与按钮状态保持一致。
    }

    private void NextTrackButton_Click(object sender, RoutedEventArgs args) { }

    private void FavoriteButton_Click(object sender, RoutedEventArgs args)
    {
        _isFavorite = !_isFavorite; // 当前只维护收藏的界面状态，持久化由播放/媒体服务接入后补上。
        FavoriteIcon.Glyph = _isFavorite ? "\uEB51" : "\uE006"; // 实心/空心图标表示收藏状态。
        ToolTipService.SetToolTip(FavoriteButton, _isFavorite ? "从我喜欢歌单移除" : "添加到我喜欢歌单"); // tooltip 说明下一次点击动作。
    }

    private void EqualizerButton_Click(object sender, RoutedEventArgs args) { }
    private void OutputDeviceButton_Click(object sender, RoutedEventArgs args) { }
    private void PlaybackProgressSlider_ValueChanged(object sender, RangeBaseValueChangedEventArgs args) { }

    private void VolumeSlider_ValueChanged(object sender, RangeBaseValueChangedEventArgs args)
    {
        VolumeIcon.Glyph = args.NewValue <= 0.001 ? "\uE992" : args.NewValue < 0.33 ? "\uE993" : args.NewValue < 0.66 ? "\uE994" : "\uE995"; // 当前仅按音量区间更新图标，音频输出命令尚未接入这里。
    }

    private ThemeMaterial ReadMaterial() => ReadString(MaterialKey)?.ToLowerInvariant() switch // 兼容历史字符串格式并转换为枚举。
    {
        "acrylic" => ThemeMaterial.Acrylic,
        "transparentacrylic" or "transparent-acrylic" => ThemeMaterial.TransparentAcrylic,
        "customacrylic" or "custom-acrylic" => ThemeMaterial.CustomAcrylic,
        "micaalt" or "mica-alt" => ThemeMaterial.MicaAlt,
        "none" => ThemeMaterial.None,
        _ => ThemeMaterial.Mica,
    };

    private ThemeSystemVariant ReadVariant()
    {
        if (Enum.TryParse(ReadString(VariantKey), true, out ThemeSystemVariant variant)) // 优先使用用户保存的模式。
        {
            return variant;
        }
        try
        {
            var color = new UISettings().GetColorValue(UIColorType.Background); // 没有保存值时读取系统背景色。
            return color.R * 0.2126 + color.G * 0.7152 + color.B * 0.0722 < 128
                ? ThemeSystemVariant.Dark
                : ThemeSystemVariant.Light;
        }
        catch (Exception)
        {
            return ThemeSystemVariant.Light;
        }
    }

    private void SaveThemeSettings()
    {
        if (_settings is null)
        {
            return;
        }

        try
        {
            _settings.Values[MaterialKey] = _material.ToString(); // 保存预设身份，而不是把所有 Acrylic 折叠成一个名称。
            _settings.Values[VariantKey] = _variant.ToString(); // 保存下次启动要使用的明暗模式。
            _settings.Values[AcrylicTransparencyKey] = _acrylicTransparency; // 保存通透度滑块原始值。
            _settings.Values[AcrylicVividnessKey] = _acrylicVividness; // 保存鲜艳度滑块原始值。
        }
        catch (Exception)
        {
            // 写盘失败不应影响 UI；滑块事件仍可继续驱动内存中的主题状态。
        }
    }

    private string? ReadString(string key) =>
        _settings?.Values.TryGetValue(key, out var value) == true ? value as string : null; // LocalSettings 值不是字符串时按缺失处理。

    private double ReadDouble(string key, double fallback)
    {
        if (_settings?.Values.TryGetValue(key, out var value) != true) return fallback; // 缺失配置直接使用调用方默认值。
        var result = value switch
        {
            double number => number,
            float number => number,
            int number => number,
            string text when double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var number) => number,
            _ => fallback,
        };
        return double.IsFinite(result) ? Math.Clamp(result, 0, 1) : fallback; // 防止旧配置或损坏值把 controller 参数带出范围。
    }

    private int? ReadInt(string key) => _settings?.Values.TryGetValue(key, out var value) == true // 窗口尺寸只接受 LocalSettings 中的 int/long 值。
        ? value switch { int number => number, long number => (int)number, _ => null }
        : null;

    private void WriteInt(string key, int value)
    {
        try
        {
            if (_settings is not null) _settings.Values[key] = value; // 尺寸保存失败时保持静默，不影响窗口关闭。
        }
        catch (Exception)
        {
        }
    }

    private static int Clamp(int value, int min, int max) => Math.Clamp(value, min, Math.Max(min, max)); // 处理显示器工作区小于最小尺寸的极端情况。

    private static ApplicationDataContainer? TryGetSettings()
    {
        try { return ApplicationData.Current.LocalSettings; } // 使用应用本地范围保存窗口和主题设置。
        catch (Exception) { return null; } // 当前环境不支持时由调用方使用默认值。
    }
}

public enum PlaybackMode
{
    ListRepeat,
    Shuffle,
    SingleTrack,
}
