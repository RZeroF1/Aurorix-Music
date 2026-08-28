using System;
using System.Runtime.InteropServices;

namespace Aurorix.Windows.Interop;

internal static class AurorixNativeMethods
{
    internal const string LibraryName = "aurorix_ffi_c";

    [StructLayout(LayoutKind.Sequential)]
    internal struct ByteSlice
    {
        internal IntPtr Pointer;
        internal ulong Length;

        internal ByteSlice(IntPtr pointer, ulong length)
        {
            Pointer = pointer;
            Length = length;
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct Buffer
    {
        internal IntPtr Pointer;
        internal ulong Length;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct ClientConfig
    {
        internal ByteSlice DataDirectory;
        internal uint ShutdownTimeoutMilliseconds;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct Error
    {
        internal int Code;
        internal Buffer Message;
    }

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void CompletionCallback(
        IntPtr context,
        int status,
        int outcome,
        ByteSlice response);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void EventSinkCallback(
        IntPtr context,
        ulong eventSequence,
        ByteSlice @event);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr aurorix_client_create_v1(
        in ClientConfig config,
        out Error error);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_client_command_v1(
        IntPtr client,
        ByteSlice request,
        CompletionCallback? callback,
        IntPtr context,
        out IntPtr operation);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_client_query_v1(
        IntPtr client,
        ByteSlice request,
        CompletionCallback? callback,
        IntPtr context,
        out IntPtr operation);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_client_subscribe_v1(
        IntPtr client,
        ByteSlice request,
        EventSinkCallback? callback,
        IntPtr context,
        out IntPtr subscription,
        out ulong observedSequence);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_operation_cancel_v1(IntPtr operation);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_operation_release_v1(IntPtr operation);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_subscription_cancel_v1(IntPtr subscription);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_subscription_release_v1(IntPtr subscription);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void aurorix_buffer_free_v1(Buffer buffer);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_client_shutdown_v1(IntPtr client);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int aurorix_client_release_v1(IntPtr client);

    internal const int StatusOk = 0;
    internal const int StatusInvalidArgument = 1;
    internal const int StatusInvalidHandle = 2;
    internal const int StatusIncompatibleVersion = 3;
    internal const int StatusShutdown = 4;
    internal const int StatusAlreadyCancelled = 5;
    internal const int StatusCancelled = 6;
    internal const int StatusCallbackRejected = 7;
    internal const int StatusPanic = 8;
    internal const int StatusShutdownIncomplete = 9;
    internal const int StatusReentrantRelease = 10;

    internal const int OutcomeCompleted = 0;
    internal const int OutcomeCancelledBeforeCommit = 1;
    internal const int OutcomeCancelledOutcomeUnknown = 2;
}
