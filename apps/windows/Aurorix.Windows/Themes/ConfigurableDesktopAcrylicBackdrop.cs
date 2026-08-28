using System;
using Microsoft.UI.Composition;
using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace Aurorix.Windows.Themes;

/// <summary>
/// Desktop Acrylic backed by a controller so tint and opacity can be changed
/// after the backdrop is attached to the window.
/// </summary>
internal sealed class ConfigurableDesktopAcrylicBackdrop : SystemBackdrop
{
    private readonly object _gate = new();
    private DesktopAcrylicController? _controller;
    private DesktopAcrylicKind _kind;
    private Color _tintColor;
    private Color _fallbackColor;
    private double _tintOpacity;
    private double _luminosityOpacity;
    private bool _controllerPropertiesApplied;
    private DesktopAcrylicKind _appliedKind;
    private Color _appliedTintColor;
    private Color _appliedFallbackColor;
    private double _appliedTintOpacity;
    private double _appliedLuminosityOpacity;

    public ConfigurableDesktopAcrylicBackdrop(
        DesktopAcrylicKind kind,
        Color tintColor,
        double tintOpacity,
        double luminosityOpacity,
        Color fallbackColor)
    {
        Update(kind, tintColor, tintOpacity, luminosityOpacity, fallbackColor);
    }

    public void Update(
        DesktopAcrylicKind kind,
        Color tintColor,
        double tintOpacity,
        double luminosityOpacity,
        Color fallbackColor)
    {
        lock (_gate)
        {
            _kind = kind;
            _tintColor = tintColor;
            _tintOpacity = Math.Clamp(tintOpacity, 0, 1);
            _luminosityOpacity = Math.Clamp(luminosityOpacity, 0, 1);
            _fallbackColor = fallbackColor;

            // Slider changes can arrive while WinUI is disconnecting the
            // backdrop. Never let a stale controller exception cross the
            // XAML event boundary.
            TryApplyControllerProperties();
        }
    }

    protected override void OnTargetConnected(
        ICompositionSupportsSystemBackdrop connectedTarget,
        XamlRoot xamlRoot)
    {
        try
        {
            base.OnTargetConnected(connectedTarget, xamlRoot);

            lock (_gate)
            {
                // WinUI can report a reconnect before the matching
                // disconnect during rapid material/theme changes. Reusing
                // the live controller is safe for this single-window host.
                if (_controller is not null)
                {
                    TrySetSystemBackdropConfiguration(connectedTarget, xamlRoot);
                    return;
                }

                var controller = new DesktopAcrylicController();
                _controller = controller;
                _controllerPropertiesApplied = false;
                try
                {
                    ApplyControllerProperties(controller);
                    controller.AddSystemBackdropTarget(connectedTarget);
                    controller.SetSystemBackdropConfiguration(
                        GetDefaultSystemBackdropConfiguration(connectedTarget, xamlRoot));
                }
                catch (Exception)
                {
                    if (ReferenceEquals(_controller, controller))
                    {
                        _controller = null;
                        _controllerPropertiesApplied = false;
                    }
                    TryDisposeController(controller, connectedTarget);
                }
            }
        }
        catch (Exception)
        {
            // WinUI invokes these callbacks; isolate managed exceptions so
            // they do not escape into XAML dispatch. Native access violations
            // are outside the protection provided by this catch block.
        }
    }

    protected override void OnTargetDisconnected(
        ICompositionSupportsSystemBackdrop disconnectedTarget)
    {
        try
        {
            base.OnTargetDisconnected(disconnectedTarget);
        }
        catch (Exception)
        {
        }

        DesktopAcrylicController? controller;
        lock (_gate)
        {
            controller = _controller;
            _controller = null;
            _controllerPropertiesApplied = false;
        }

        if (controller is null)
        {
            return;
        }

        // RemoveSystemBackdropTarget is not guaranteed to be idempotent;
        // rapid SystemBackdrop replacement can make it report
        // E_ELEMENTNOTFOUND. Both removal and disposal are best-effort.
        try
        {
            controller.RemoveSystemBackdropTarget(disconnectedTarget);
        }
        catch (Exception)
        {
        }

        try
        {
            controller.Dispose();
        }
        catch (Exception)
        {
        }
    }

    protected override void OnDefaultSystemBackdropConfigurationChanged(
        ICompositionSupportsSystemBackdrop target,
        XamlRoot xamlRoot)
    {
        try
        {
            base.OnDefaultSystemBackdropConfigurationChanged(target, xamlRoot);
        }
        catch (Exception)
        {
        }

        lock (_gate)
        {
            TrySetSystemBackdropConfiguration(target, xamlRoot);
        }
    }

    private void TrySetSystemBackdropConfiguration(
        ICompositionSupportsSystemBackdrop target,
        XamlRoot xamlRoot)
    {
        if (_controller is null)
        {
            return;
        }

        try
        {
            _controller.SetSystemBackdropConfiguration(
                GetDefaultSystemBackdropConfiguration(target, xamlRoot));
        }
        catch (Exception)
        {
        }
    }

    private void TryApplyControllerProperties()
    {
        if (_controller is null)
        {
            return;
        }

        try
        {
            ApplyControllerProperties(_controller);
        }
        catch (Exception)
        {
        }
    }

    private void ApplyControllerProperties(DesktopAcrylicController controller)
    {
        if (!_controllerPropertiesApplied || _appliedKind != _kind)
        {
            controller.Kind = _kind;
            _appliedKind = _kind;
        }

        if (!_controllerPropertiesApplied || !_appliedTintColor.Equals(_tintColor))
        {
            controller.TintColor = _tintColor;
            _appliedTintColor = _tintColor;
        }

        if (!_controllerPropertiesApplied || _appliedTintOpacity != _tintOpacity)
        {
            controller.TintOpacity = (float)_tintOpacity;
            _appliedTintOpacity = _tintOpacity;
        }

        if (!_controllerPropertiesApplied || _appliedLuminosityOpacity != _luminosityOpacity)
        {
            controller.LuminosityOpacity = (float)_luminosityOpacity;
            _appliedLuminosityOpacity = _luminosityOpacity;
        }

        if (!_controllerPropertiesApplied || !_appliedFallbackColor.Equals(_fallbackColor))
        {
            controller.FallbackColor = _fallbackColor;
            _appliedFallbackColor = _fallbackColor;
        }

        _controllerPropertiesApplied = true;
    }

    private static void TryDisposeController(
        DesktopAcrylicController controller,
        ICompositionSupportsSystemBackdrop target)
    {
        try
        {
            controller.RemoveSystemBackdropTarget(target);
        }
        catch (Exception)
        {
        }

        try
        {
            controller.Dispose();
        }
        catch (Exception)
        {
        }
    }
}
