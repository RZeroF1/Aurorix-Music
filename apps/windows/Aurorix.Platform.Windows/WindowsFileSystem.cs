namespace Aurorix.Platform.Windows;

public sealed record WindowsFileSystemEntryInfo(
    WindowsLocatorKind Kind,
    long SizeBytes,
    DateTimeOffset LastWriteTimeUtc,
    bool IsReparsePoint);

/// <summary>
/// Small boundary around System.IO so scan and relink behavior can be tested
/// without a native picker or a real user library.
/// </summary>
public interface IWindowsFileSystem
{
    WindowsFileSystemEntryInfo GetInfo(string normalizedPath);

    IEnumerable<string> EnumerateChildren(string normalizedDirectoryPath);
}

public sealed class SystemWindowsFileSystem : IWindowsFileSystem
{
    public WindowsFileSystemEntryInfo GetInfo(string normalizedPath)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(normalizedPath);

        var attributes = File.GetAttributes(normalizedPath);
        var isDirectory = attributes.HasFlag(FileAttributes.Directory);
        var isReparsePoint = attributes.HasFlag(FileAttributes.ReparsePoint);

        if (isDirectory)
        {
            var directory = new DirectoryInfo(normalizedPath);
            return new WindowsFileSystemEntryInfo(
                WindowsLocatorKind.Folder,
                0,
                directory.LastWriteTimeUtc,
                isReparsePoint);
        }

        var file = new FileInfo(normalizedPath);
        return new WindowsFileSystemEntryInfo(
            WindowsLocatorKind.File,
            file.Length,
            file.LastWriteTimeUtc,
            isReparsePoint);
    }

    public IEnumerable<string> EnumerateChildren(string normalizedDirectoryPath)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(normalizedDirectoryPath);

        return Directory.EnumerateFileSystemEntries(
            normalizedDirectoryPath,
            "*",
            new EnumerationOptions
            {
                RecurseSubdirectories = false,
                IgnoreInaccessible = false,
                ReturnSpecialDirectories = false,
                AttributesToSkip = 0,
            });
    }
}

public interface IWindowsFileIdentityProvider
{
    WindowsFileIdentity? TryGetIdentity(string normalizedPath, WindowsLocatorKind kind);
}

/// <summary>
/// Reads the Windows volume serial and file index while keeping the native
/// handle entirely inside this adapter call.
/// </summary>
public sealed class WindowsFileIdentityProvider : IWindowsFileIdentityProvider
{
    private const uint OpenExisting = 3;
    private const uint FileAttributeNormal = 0x80;
    private const uint FileFlagBackupSemantics = 0x02000000;
    private const uint FileShareRead = 0x00000001;
    private const uint FileShareWrite = 0x00000002;
    private const uint FileShareDelete = 0x00000004;

    public WindowsFileIdentity? TryGetIdentity(string normalizedPath, WindowsLocatorKind kind)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(normalizedPath);

        var flags = kind == WindowsLocatorKind.Folder ? FileFlagBackupSemantics : FileAttributeNormal;
        using var handle = CreateFileW(
            normalizedPath,
            0,
            FileShareRead | FileShareWrite | FileShareDelete,
            IntPtr.Zero,
            OpenExisting,
            flags,
            IntPtr.Zero);

        if (handle.IsInvalid || !GetFileInformationByHandle(handle, out var information))
        {
            return null;
        }

        var fileIndex = ((ulong)information.FileIndexHigh << 32) | information.FileIndexLow;
        return new WindowsFileIdentity(information.VolumeSerialNumber, fileIndex);
    }

    [System.Runtime.InteropServices.DllImport(
        "kernel32.dll",
        CharSet = System.Runtime.InteropServices.CharSet.Unicode,
        EntryPoint = "CreateFileW",
        SetLastError = true)]
    private static extern Microsoft.Win32.SafeHandles.SafeFileHandle CreateFileW(
        string fileName,
        uint desiredAccess,
        uint shareMode,
        IntPtr securityAttributes,
        uint creationDisposition,
        uint flagsAndAttributes,
        IntPtr templateFile);

    [System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError = true)]
    [return: System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.Bool)]
    private static extern bool GetFileInformationByHandle(
        Microsoft.Win32.SafeHandles.SafeFileHandle fileHandle,
        out ByHandleFileInformation fileInformation);

    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)]
    private struct ByHandleFileInformation
    {
        public uint FileAttributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME CreationTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastAccessTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWriteTime;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }
}
