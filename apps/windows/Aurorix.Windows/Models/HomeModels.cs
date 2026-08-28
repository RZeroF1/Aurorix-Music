using System;
using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace Aurorix.Windows.Models;

/// <summary>
/// The source of a home item. This is intentionally broader than the current
/// local-only fixture so provider and extension content can use the same UI.
/// </summary>
public enum HomeSourceKind
{
    Local,
    Provider,
    Extension,
    System
}

public enum HomeDataState
{
    Ready,
    Loading,
    Empty,
    Offline,
    Error
}

public enum HomeCommandKind
{
    OpenQuickEntry,
    PlayTrack,
    OpenRecommendation,
    OpenAllRecent,
    CustomizeHome,
    Search,
    ToggleFavorite,
    OpenExtension
}

/// <summary>
/// A semantic action request emitted by the Home view model. The host is
/// responsible for translating it to a Core Facade command or a navigation
/// route; the view model does not own playback or navigation state.
/// </summary>
public sealed class HomeCommandRequestedEventArgs : EventArgs
{
    public HomeCommandRequestedEventArgs(
        HomeCommandKind command,
        string itemId,
        string? routeId = null,
        string? query = null)
    {
        Command = command;
        ItemId = itemId;
        RouteId = routeId;
        Query = query;
    }

    public HomeCommandKind Command { get; }

    public string ItemId { get; }

    public string? RouteId { get; }

    public string? Query { get; }
}

public abstract class HomeItem : INotifyPropertyChanged
{
    private bool _isEnabled = true;
    private bool _isVisible = true;

    protected HomeItem(string id, string title, HomeSourceKind source)
    {
        Id = id;
        Title = title;
        Source = source;
    }

    public string Id { get; }

    public string Title { get; }

    public HomeSourceKind Source { get; }

    public bool IsEnabled
    {
        get => _isEnabled;
        set => SetField(ref _isEnabled, value);
    }

    public bool IsVisible
    {
        get => _isVisible;
        set => SetField(ref _isVisible, value);
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    protected bool SetField<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (Equals(field, value))
        {
            return false;
        }

        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
        return true;
    }
}

public sealed class HomeQuickEntry : HomeItem
{
    public HomeQuickEntry(
        string id,
        string title,
        string subtitle,
        string iconGlyph,
        HomeSourceKind source = HomeSourceKind.Local,
        string? routeId = null,
        int? count = null,
        bool isCustomizable = false)
        : base(id, title, source)
    {
        Subtitle = subtitle;
        IconGlyph = iconGlyph;
        RouteId = routeId ?? id;
        Count = count;
        IsCustomizable = isCustomizable;
    }

    public string Subtitle { get; }

    /// <summary>
    /// A Segoe Fluent Icons glyph code. The host may replace it with a plugin
    /// icon reference once the extension manifest contract is available.
    /// </summary>
    public string IconGlyph { get; }

    public string RouteId { get; }

    public int? Count { get; }

    public bool HasCount => Count.HasValue;

    public bool IsCustomizable { get; }
}

public sealed class HomeRecentTrack : HomeItem
{
    private bool _isFavorite;

    public HomeRecentTrack(
        string id,
        string title,
        string artist,
        string album,
        string duration,
        string artworkKey,
        string qualityLabel,
        bool isFavorite = false,
        HomeSourceKind source = HomeSourceKind.Local)
        : base(id, title, source)
    {
        Artist = artist;
        Album = album;
        Duration = duration;
        ArtworkKey = artworkKey;
        QualityLabel = qualityLabel;
        _isFavorite = isFavorite;
    }

    public string Artist { get; }

    public string Album { get; }

    public string Duration { get; }

    /// <summary>
    /// A logical artwork key, not a file path. The presentation layer resolves
    /// it through the future artwork/cache facade.
    /// </summary>
    public string ArtworkKey { get; }

    public string QualityLabel { get; }

    public bool IsFavorite
    {
        get => _isFavorite;
        set => SetField(ref _isFavorite, value);
    }
}

public sealed class HomeRecommendationCard : HomeItem
{
    public HomeRecommendationCard(
        string id,
        string title,
        string description,
        string artworkKey,
        string routeId,
        HomeSourceKind source = HomeSourceKind.Local)
        : base(id, title, source)
    {
        Description = description;
        ArtworkKey = artworkKey;
        RouteId = routeId;
    }

    public string Description { get; }

    public string ArtworkKey { get; }

    public string RouteId { get; }
}

/// <summary>
/// Declarative extension contribution rendered by the host. It provides the
/// future plugin surface without allowing arbitrary XAML or visual-tree edits.
/// </summary>
public sealed class HomeExtensionContribution : HomeItem
{
    public HomeExtensionContribution(
        string id,
        string title,
        string slotId,
        string iconGlyph,
        string? routeId = null,
        HomeSourceKind source = HomeSourceKind.Extension)
        : base(id, title, source)
    {
        SlotId = slotId;
        IconGlyph = iconGlyph;
        RouteId = routeId ?? id;
    }

    public string SlotId { get; }

    public string IconGlyph { get; }

    public string RouteId { get; }
}
