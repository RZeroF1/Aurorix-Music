using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace Aurorix.Windows.Shell;

public sealed partial class ShellPlaceholderPage : Page
{
    public ShellRoute Route { get; private set; } = ShellNavigation.Home;

    public ShellPlaceholderPage()
    {
        InitializeComponent();
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);

        Route = e.Parameter as ShellRoute ?? ShellNavigation.Home;
        PageTitleText.Text = Route.Title;
        PageDescriptionText.Text = Route.Description;
        AutomationProperties.SetName(this, $"{Route.Title}页面");
    }
}
