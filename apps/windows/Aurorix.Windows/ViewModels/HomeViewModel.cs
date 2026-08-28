using System;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows.Input;
using Aurorix.Windows.Models;

namespace Aurorix.Windows.ViewModels;

public sealed class HomeViewModel : INotifyPropertyChanged
{
    private string _greeting = "Good Morning, Aurorix";
    private string _greetingSubtitle = "准备好享受今天的音乐了吗？";
    private HomeDataState _state = HomeDataState.Ready;
    private string _searchQuery = string.Empty;

    public HomeViewModel()
    {
        QuickEntries = CreateQuickEntries();
        RecentTracks = CreateRecentTracks();
        Recommendations = CreateRecommendations();
        ExtensionContributions = new ObservableCollection<HomeExtensionContribution>();

        OpenQuickEntryCommand = new RelayCommand<HomeQuickEntry>(
            entry =>
            {
                if (entry is not null)
                {
                    Request(
                        HomeCommandKind.OpenQuickEntry,
                        entry.Id,
                        entry.RouteId);
                }
            },
            entry => entry is not null && entry.IsEnabled);
        PlayTrackCommand = new RelayCommand<HomeRecentTrack>(
            track =>
            {
                if (track is not null)
                {
                    Request(HomeCommandKind.PlayTrack, track.Id);
                }
            },
            track => track is not null && track.IsEnabled);
        OpenRecommendationCommand = new RelayCommand<HomeRecommendationCard>(
            recommendation =>
            {
                if (recommendation is not null)
                {
                    Request(
                        HomeCommandKind.OpenRecommendation,
                        recommendation.Id,
                        recommendation.RouteId);
                }
            },
            recommendation => recommendation is not null && recommendation.IsEnabled);
        OpenAllRecentCommand = new RelayCommand(
            () => Request(HomeCommandKind.OpenAllRecent, "recently-played"));
        CustomizeHomeCommand = new RelayCommand(
            () => Request(HomeCommandKind.CustomizeHome, "home"));
        SearchCommand = new RelayCommand<string?>(
            query =>
            {
                SearchQuery = query?.Trim() ?? string.Empty;
                Request(HomeCommandKind.Search, "global-search", query: SearchQuery);
            });
        ToggleFavoriteCommand = new RelayCommand<HomeRecentTrack>(
            track =>
            {
                if (track is not null)
                {
                    Request(HomeCommandKind.ToggleFavorite, track.Id);
                }
            },
            track => track is not null && track.IsEnabled);
        OpenExtensionCommand = new RelayCommand<HomeExtensionContribution>(
            contribution =>
            {
                if (contribution is not null)
                {
                    Request(
                        HomeCommandKind.OpenExtension,
                        contribution.Id,
                        contribution.RouteId);
                }
            },
            contribution => contribution is not null && contribution.IsEnabled);
    }

    public string Greeting
    {
        get => _greeting;
        set => SetField(ref _greeting, value);
    }

    public string GreetingSubtitle
    {
        get => _greetingSubtitle;
        set => SetField(ref _greetingSubtitle, value);
    }

    public HomeDataState State
    {
        get => _state;
        set
        {
            if (!SetField(ref _state, value))
            {
                return;
            }

            OnPropertyChanged(nameof(IsLoading));
            OnPropertyChanged(nameof(IsOffline));
            OnPropertyChanged(nameof(HasContent));
        }
    }

    public bool IsLoading => State == HomeDataState.Loading;

    public bool IsOffline => State == HomeDataState.Offline;

    public bool HasContent => State == HomeDataState.Ready &&
        (QuickEntries.Count > 0 || RecentTracks.Count > 0 || Recommendations.Count > 0);

    public string SearchQuery
    {
        get => _searchQuery;
        set => SetField(ref _searchQuery, value);
    }

    public ObservableCollection<HomeQuickEntry> QuickEntries { get; }

    public ObservableCollection<HomeRecentTrack> RecentTracks { get; }

    public ObservableCollection<HomeRecommendationCard> Recommendations { get; }

    /// <summary>
    /// Contributions are empty in the local fixture but remain observable so
    /// an extension registry can add validated declarations later.
    /// </summary>
    public ObservableCollection<HomeExtensionContribution> ExtensionContributions { get; }

    public ICommand OpenQuickEntryCommand { get; }

    public ICommand PlayTrackCommand { get; }

    // Alias kept for XAML readability when the command is bound from a recent-track card.
    public ICommand PlayRecentTrackCommand => PlayTrackCommand;

    public ICommand OpenRecommendationCommand { get; }

    public ICommand OpenAllRecentCommand { get; }

    public ICommand CustomizeHomeCommand { get; }

    public ICommand SearchCommand { get; }

    public ICommand ToggleFavoriteCommand { get; }

    public ICommand OpenExtensionCommand { get; }

    public event EventHandler<HomeCommandRequestedEventArgs>? CommandRequested;

    public event PropertyChangedEventHandler? PropertyChanged;

    private void Request(
        HomeCommandKind command,
        string itemId,
        string? routeId = null,
        string? query = null)
    {
        CommandRequested?.Invoke(
            this,
            new HomeCommandRequestedEventArgs(command, itemId, routeId, query));
    }

    private static ObservableCollection<HomeQuickEntry> CreateQuickEntries()
    {
        return new ObservableCollection<HomeQuickEntry>
        {
            new("favorites", "我喜欢的音乐", "128 首歌曲", "\uE734", count: 128),
            new("recently-played", "最近播放", "356 首歌曲", "\uE823", count: 356),
            new("downloads", "下载管理", "42 首歌曲", "\uE896", count: 42),
            new("playlists", "我的歌单", "8 个歌单", "\uE8A5", count: 8),
            new("custom", "自定义", "添加快捷入口", "\uE710", isCustomizable: true)
        };
    }

    private static ObservableCollection<HomeRecentTrack> CreateRecentTracks()
    {
        return new ObservableCollection<HomeRecentTrack>
        {
            new(
                "track-night-seventh-chapter",
                "夜的第七章",
                "周杰伦",
                "叶惠美",
                "04:45",
                "night-seventh-chapter",
                "FLAC 24bit / 96kHz",
                isFavorite: true),
            new(
                "track-warm-summer",
                "温柔星球",
                "小王子",
                "温柔星球",
                "03:58",
                "warm-summer",
                "FLAC 24bit / 48kHz"),
            new(
                "track-renaissance",
                "Renaissance",
                "Beyonce",
                "Renaissance",
                "04:05",
                "renaissance",
                "AAC 256kbps"),
            new(
                "track-my-sunshine",
                "我的日常",
                "RADWIMPS",
                "君の名は。",
                "04:12",
                "my-sunshine",
                "FLAC 24bit / 48kHz"),
            new(
                "track-hotel-california",
                "Hotel California",
                "Eagles",
                "Hotel California",
                "06:30",
                "hotel-california",
                "FLAC 24bit / 44.1kHz")
        };
    }

    private static ObservableCollection<HomeRecommendationCard> CreateRecommendations()
    {
        return new ObservableCollection<HomeRecommendationCard>
        {
            new(
                "recommendation-daily-mix",
                "每日推荐",
                "根据你的听歌习惯精选",
                "daily-mix",
                "playlist:daily-mix"),
            new(
                "recommendation-new-releases",
                "新歌速递",
                "最新发行的热门歌曲",
                "new-releases",
                "discover:new-releases"),
            new(
                "recommendation-mood-radio",
                "氛围电台",
                "随音乐进入专注时刻",
                "mood-radio",
                "radio:mood")
        };
    }

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (Equals(field, value))
        {
            return false;
        }

        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null)
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }
}

public sealed class RelayCommand : ICommand
{
    private readonly Action _execute;
    private readonly Func<bool>? _canExecute;

    public RelayCommand(Action execute, Func<bool>? canExecute = null)
    {
        _execute = execute ?? throw new ArgumentNullException(nameof(execute));
        _canExecute = canExecute;
    }

    public event EventHandler? CanExecuteChanged;

    public bool CanExecute(object? parameter) => _canExecute?.Invoke() ?? true;

    public void Execute(object? parameter) => _execute();

    public void NotifyCanExecuteChanged() => CanExecuteChanged?.Invoke(this, EventArgs.Empty);
}

public sealed class RelayCommand<T> : ICommand
{
    private readonly Action<T?> _execute;
    private readonly Predicate<T?>? _canExecute;

    public RelayCommand(Action<T?> execute, Predicate<T?>? canExecute = null)
    {
        _execute = execute ?? throw new ArgumentNullException(nameof(execute));
        _canExecute = canExecute;
    }

    public event EventHandler? CanExecuteChanged;

    public bool CanExecute(object? parameter)
    {
        return _canExecute?.Invoke(ConvertParameter(parameter)) ?? true;
    }

    public void Execute(object? parameter) => _execute(ConvertParameter(parameter));

    public void NotifyCanExecuteChanged() => CanExecuteChanged?.Invoke(this, EventArgs.Empty);

    private static T? ConvertParameter(object? parameter)
    {
        return parameter is T value ? value : default;
    }
}
