using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Aurorix.Windows.ViewModels;

namespace Aurorix.Windows.Home;

public sealed partial class HomePage : Page
{
    public HomeViewModel ViewModel { get; } = new();

    public HomePage()
    {
        InitializeComponent();
        DataContext = ViewModel;
        ViewModel.CommandRequested += OnCommandRequested;
    }

    private void OnCommandRequested(object? sender, Models.HomeCommandRequestedEventArgs args)
    {
        // The shell will route these semantic requests to the Core facade once
        // the Gate 3 transport is available. Fixture UI stays usable meanwhile.
    }

    private void SearchBox_QuerySubmitted(AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args)
    {
        if (ViewModel.SearchCommand.CanExecute(sender.Text))
        {
            ViewModel.SearchCommand.Execute(sender.Text);
        }
    }
}
