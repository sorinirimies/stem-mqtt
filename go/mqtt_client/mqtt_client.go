package mqtt_client

// #cgo CFLAGS: -I${SRCDIR}
// #cgo LDFLAGS: -lmqtt_client
// #include <mqtt_client.h>
import "C"

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"math"
	"reflect"
	"runtime"
	"runtime/cgo"
	"sync"
	"sync/atomic"
	"unsafe"
)

// This is needed, because as of go 1.24
// type RustBuffer C.RustBuffer cannot have methods,
// RustBuffer is treated as non-local type
type GoRustBuffer struct {
	inner C.RustBuffer
}

type RustBufferI interface {
	AsReader() *bytes.Reader
	Free()
	ToGoBytes() []byte
	Data() unsafe.Pointer
	Len() uint64
	Capacity() uint64
}

// C.RustBuffer fields exposed as an interface so they can be accessed in different Go packages.
// See https://github.com/golang/go/issues/13467
type ExternalCRustBuffer interface {
	Data() unsafe.Pointer
	Len() uint64
	Capacity() uint64
}

func RustBufferFromC(b C.RustBuffer) ExternalCRustBuffer {
	return GoRustBuffer{
		inner: b,
	}
}

func CFromRustBuffer(b ExternalCRustBuffer) C.RustBuffer {
	return C.RustBuffer{
		capacity: C.uint64_t(b.Capacity()),
		len:      C.uint64_t(b.Len()),
		data:     (*C.uchar)(b.Data()),
	}
}

func RustBufferFromExternal(b ExternalCRustBuffer) GoRustBuffer {
	return GoRustBuffer{
		inner: C.RustBuffer{
			capacity: C.uint64_t(b.Capacity()),
			len:      C.uint64_t(b.Len()),
			data:     (*C.uchar)(b.Data()),
		},
	}
}

func (cb GoRustBuffer) Capacity() uint64 {
	return uint64(cb.inner.capacity)
}

func (cb GoRustBuffer) Len() uint64 {
	return uint64(cb.inner.len)
}

func (cb GoRustBuffer) Data() unsafe.Pointer {
	return unsafe.Pointer(cb.inner.data)
}

func (cb GoRustBuffer) AsReader() *bytes.Reader {
	b := unsafe.Slice((*byte)(cb.inner.data), C.uint64_t(cb.inner.len))
	return bytes.NewReader(b)
}

func (cb GoRustBuffer) Free() {
	rustCall(func(status *C.RustCallStatus) bool {
		C.ffi_mqtt_client_rustbuffer_free(cb.inner, status)
		return false
	})
}

func (cb GoRustBuffer) ToGoBytes() []byte {
	return C.GoBytes(unsafe.Pointer(cb.inner.data), C.int(cb.inner.len))
}

func stringToRustBuffer(str string) C.RustBuffer {
	return bytesToRustBuffer([]byte(str))
}

func bytesToRustBuffer(b []byte) C.RustBuffer {
	if len(b) == 0 {
		return C.RustBuffer{}
	}
	// We can pass the pointer along here, as it is pinned
	// for the duration of this call
	foreign := C.ForeignBytes{
		len:  C.int(len(b)),
		data: (*C.uchar)(unsafe.Pointer(&b[0])),
	}

	return rustCall(func(status *C.RustCallStatus) C.RustBuffer {
		return C.ffi_mqtt_client_rustbuffer_from_bytes(foreign, status)
	})
}

type BufLifter[GoType any] interface {
	Lift(value RustBufferI) GoType
}

type BufLowerer[GoType any] interface {
	Lower(value GoType) C.RustBuffer
}

type BufReader[GoType any] interface {
	Read(reader io.Reader) GoType
}

type BufWriter[GoType any] interface {
	Write(writer io.Writer, value GoType)
}

func LowerIntoRustBuffer[GoType any](bufWriter BufWriter[GoType], value GoType) C.RustBuffer {
	// This might be not the most efficient way but it does not require knowing allocation size
	// beforehand
	var buffer bytes.Buffer
	bufWriter.Write(&buffer, value)

	bytes, err := io.ReadAll(&buffer)
	if err != nil {
		panic(fmt.Errorf("reading written data: %w", err))
	}
	return bytesToRustBuffer(bytes)
}

func LiftFromRustBuffer[GoType any](bufReader BufReader[GoType], rbuf RustBufferI) GoType {
	defer rbuf.Free()
	reader := rbuf.AsReader()
	item := bufReader.Read(reader)
	if reader.Len() > 0 {
		// TODO: Remove this
		leftover, _ := io.ReadAll(reader)
		panic(fmt.Errorf("Junk remaining in buffer after lifting: %s", string(leftover)))
	}
	return item
}

func rustCallWithError[E any, U any](converter BufReader[E], callback func(*C.RustCallStatus) U) (U, E) {
	var status C.RustCallStatus
	returnValue := callback(&status)
	err := checkCallStatus(converter, status)
	return returnValue, err
}

func checkCallStatus[E any](converter BufReader[E], status C.RustCallStatus) E {
	switch status.code {
	case 0:
		var zero E
		return zero
	case 1:
		return LiftFromRustBuffer(converter, GoRustBuffer{inner: status.errorBuf})
	case 2:
		// when the rust code sees a panic, it tries to construct a rustBuffer
		// with the message.  but if that code panics, then it just sends back
		// an empty buffer.
		if status.errorBuf.len > 0 {
			panic(fmt.Errorf("%s", FfiConverterStringINSTANCE.Lift(GoRustBuffer{inner: status.errorBuf})))
		} else {
			panic(fmt.Errorf("Rust panicked while handling Rust panic"))
		}
	default:
		panic(fmt.Errorf("unknown status code: %d", status.code))
	}
}

func checkCallStatusUnknown(status C.RustCallStatus) error {
	switch status.code {
	case 0:
		return nil
	case 1:
		panic(fmt.Errorf("function not returning an error returned an error"))
	case 2:
		// when the rust code sees a panic, it tries to construct a C.RustBuffer
		// with the message.  but if that code panics, then it just sends back
		// an empty buffer.
		if status.errorBuf.len > 0 {
			panic(fmt.Errorf("%s", FfiConverterStringINSTANCE.Lift(GoRustBuffer{
				inner: status.errorBuf,
			})))
		} else {
			panic(fmt.Errorf("Rust panicked while handling Rust panic"))
		}
	default:
		return fmt.Errorf("unknown status code: %d", status.code)
	}
}

func rustCall[U any](callback func(*C.RustCallStatus) U) U {
	returnValue, err := rustCallWithError[error](nil, callback)
	if err != nil {
		panic(err)
	}
	return returnValue
}

type NativeError interface {
	AsError() error
}

func writeInt8(writer io.Writer, value int8) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint8(writer io.Writer, value uint8) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt16(writer io.Writer, value int16) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint16(writer io.Writer, value uint16) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt32(writer io.Writer, value int32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint32(writer io.Writer, value uint32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt64(writer io.Writer, value int64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint64(writer io.Writer, value uint64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeFloat32(writer io.Writer, value float32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeFloat64(writer io.Writer, value float64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func readInt8(reader io.Reader) int8 {
	var result int8
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint8(reader io.Reader) uint8 {
	var result uint8
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt16(reader io.Reader) int16 {
	var result int16
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint16(reader io.Reader) uint16 {
	var result uint16
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt32(reader io.Reader) int32 {
	var result int32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint32(reader io.Reader) uint32 {
	var result uint32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt64(reader io.Reader) int64 {
	var result int64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint64(reader io.Reader) uint64 {
	var result uint64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readFloat32(reader io.Reader) float32 {
	var result float32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readFloat64(reader io.Reader) float64 {
	var result float64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func init() {

	FfiConverterMqttAuthHandlerINSTANCE.register()
	FfiConverterMqttMessageListenerINSTANCE.register()
	uniffiCheckChecksums()
}

func uniffiCheckChecksums() {
	// Get the bindings contract version from our ComponentInterface
	bindingsContractVersion := 30
	// Get the scaffolding contract version by calling the into the dylib
	scaffoldingContractVersion := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.ffi_mqtt_client_uniffi_contract_version()
	})
	if bindingsContractVersion != int(scaffoldingContractVersion) {
		// If this happens try cleaning and rebuilding your project
		panic("mqtt_client: UniFFI contract version mismatch")
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_connect()
		})
		if checksum != 57287 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_connect: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_disconnect()
		})
		if checksum != 25891 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_disconnect: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_enable_message_queue()
		})
		if checksum != 30293 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_enable_message_queue: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_is_connected()
		})
		if checksum != 59903 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_is_connected: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_last_disconnect_reason()
		})
		if checksum != 54850 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_last_disconnect_reason: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_next_message()
		})
		if checksum != 55769 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_next_message: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_publish()
		})
		if checksum != 22052 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_publish: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_set_auth_handler()
		})
		if checksum != 59903 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_set_auth_handler: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_set_message_listener()
		})
		if checksum != 56130 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_set_message_listener: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_subscribe()
		})
		if checksum != 17479 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_subscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttclient_unsubscribe()
		})
		if checksum != 3320 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttclient_unsubscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttauthhandler_respond()
		})
		if checksum != 51113 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttauthhandler_respond: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttmessagelistener_on_message()
		})
		if checksum != 46832 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttmessagelistener_on_message: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_method_mqttmessagelistener_on_disconnected()
		})
		if checksum != 46846 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_method_mqttmessagelistener_on_disconnected: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_client_checksum_constructor_mqttclient_new()
		})
		if checksum != 56208 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_client: uniffi_mqtt_client_checksum_constructor_mqttclient_new: UniFFI API checksum mismatch")
		}
	}
}

type FfiConverterUint8 struct{}

var FfiConverterUint8INSTANCE = FfiConverterUint8{}

func (FfiConverterUint8) Lower(value uint8) C.uint8_t {
	return C.uint8_t(value)
}

func (FfiConverterUint8) Write(writer io.Writer, value uint8) {
	writeUint8(writer, value)
}

func (FfiConverterUint8) Lift(value C.uint8_t) uint8 {
	return uint8(value)
}

func (FfiConverterUint8) Read(reader io.Reader) uint8 {
	return readUint8(reader)
}

type FfiDestroyerUint8 struct{}

func (FfiDestroyerUint8) Destroy(_ uint8) {}

type FfiConverterUint16 struct{}

var FfiConverterUint16INSTANCE = FfiConverterUint16{}

func (FfiConverterUint16) Lower(value uint16) C.uint16_t {
	return C.uint16_t(value)
}

func (FfiConverterUint16) Write(writer io.Writer, value uint16) {
	writeUint16(writer, value)
}

func (FfiConverterUint16) Lift(value C.uint16_t) uint16 {
	return uint16(value)
}

func (FfiConverterUint16) Read(reader io.Reader) uint16 {
	return readUint16(reader)
}

type FfiDestroyerUint16 struct{}

func (FfiDestroyerUint16) Destroy(_ uint16) {}

type FfiConverterUint32 struct{}

var FfiConverterUint32INSTANCE = FfiConverterUint32{}

func (FfiConverterUint32) Lower(value uint32) C.uint32_t {
	return C.uint32_t(value)
}

func (FfiConverterUint32) Write(writer io.Writer, value uint32) {
	writeUint32(writer, value)
}

func (FfiConverterUint32) Lift(value C.uint32_t) uint32 {
	return uint32(value)
}

func (FfiConverterUint32) Read(reader io.Reader) uint32 {
	return readUint32(reader)
}

type FfiDestroyerUint32 struct{}

func (FfiDestroyerUint32) Destroy(_ uint32) {}

type FfiConverterBool struct{}

var FfiConverterBoolINSTANCE = FfiConverterBool{}

func (FfiConverterBool) Lower(value bool) C.int8_t {
	if value {
		return C.int8_t(1)
	}
	return C.int8_t(0)
}

func (FfiConverterBool) Write(writer io.Writer, value bool) {
	if value {
		writeInt8(writer, 1)
	} else {
		writeInt8(writer, 0)
	}
}

func (FfiConverterBool) Lift(value C.int8_t) bool {
	return value != 0
}

func (FfiConverterBool) Read(reader io.Reader) bool {
	return readInt8(reader) != 0
}

type FfiDestroyerBool struct{}

func (FfiDestroyerBool) Destroy(_ bool) {}

type FfiConverterString struct{}

var FfiConverterStringINSTANCE = FfiConverterString{}

func (FfiConverterString) Lift(rb RustBufferI) string {
	defer rb.Free()
	reader := rb.AsReader()
	b, err := io.ReadAll(reader)
	if err != nil {
		panic(fmt.Errorf("reading reader: %w", err))
	}
	return string(b)
}

func (FfiConverterString) Read(reader io.Reader) string {
	length := readInt32(reader)
	buffer := make([]byte, length)
	read_length, err := reader.Read(buffer)
	if err != nil && err != io.EOF {
		panic(err)
	}
	if read_length != int(length) {
		panic(fmt.Errorf("bad read length when reading string, expected %d, read %d", length, read_length))
	}
	return string(buffer)
}

func (FfiConverterString) Lower(value string) C.RustBuffer {
	return stringToRustBuffer(value)
}

func (c FfiConverterString) LowerExternal(value string) ExternalCRustBuffer {
	return RustBufferFromC(stringToRustBuffer(value))
}

func (FfiConverterString) Write(writer io.Writer, value string) {
	if len(value) > math.MaxInt32 {
		panic("String is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	write_length, err := io.WriteString(writer, value)
	if err != nil {
		panic(err)
	}
	if write_length != len(value) {
		panic(fmt.Errorf("bad write length when writing string, expected %d, written %d", len(value), write_length))
	}
}

type FfiDestroyerString struct{}

func (FfiDestroyerString) Destroy(_ string) {}

type FfiConverterBytes struct{}

var FfiConverterBytesINSTANCE = FfiConverterBytes{}

func (c FfiConverterBytes) Lower(value []byte) C.RustBuffer {
	return LowerIntoRustBuffer[[]byte](c, value)
}

func (c FfiConverterBytes) LowerExternal(value []byte) ExternalCRustBuffer {
	return RustBufferFromC(c.Lower(value))
}

func (c FfiConverterBytes) Write(writer io.Writer, value []byte) {
	if len(value) > math.MaxInt32 {
		panic("[]byte is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	write_length, err := writer.Write(value)
	if err != nil {
		panic(err)
	}
	if write_length != len(value) {
		panic(fmt.Errorf("bad write length when writing []byte, expected %d, written %d", len(value), write_length))
	}
}

func (c FfiConverterBytes) Lift(rb RustBufferI) []byte {
	return LiftFromRustBuffer[[]byte](c, rb)
}

func (c FfiConverterBytes) Read(reader io.Reader) []byte {
	length := readInt32(reader)
	buffer := make([]byte, length)
	read_length, err := reader.Read(buffer)
	if err != nil && err != io.EOF {
		panic(err)
	}
	if read_length != int(length) {
		panic(fmt.Errorf("bad read length when reading []byte, expected %d, read %d", length, read_length))
	}
	return buffer
}

type FfiDestroyerBytes struct{}

func (FfiDestroyerBytes) Destroy(_ []byte) {}

// Below is an implementation of synchronization requirements outlined in the link.
// https://github.com/mozilla/uniffi-rs/blob/0dc031132d9493ca812c3af6e7dd60ad2ea95bf0/uniffi_bindgen/src/bindings/kotlin/templates/ObjectRuntime.kt#L31

type FfiObject struct {
	handle        C.uint64_t
	callCounter   atomic.Int64
	cloneFunction func(C.uint64_t, *C.RustCallStatus) C.uint64_t
	freeFunction  func(C.uint64_t, *C.RustCallStatus)
	destroyed     atomic.Bool
}

func newFfiObject(
	handle C.uint64_t,
	cloneFunction func(C.uint64_t, *C.RustCallStatus) C.uint64_t,
	freeFunction func(C.uint64_t, *C.RustCallStatus),
) FfiObject {
	return FfiObject{
		handle:        handle,
		cloneFunction: cloneFunction,
		freeFunction:  freeFunction,
	}
}

func (ffiObject *FfiObject) incrementPointer(debugName string) C.uint64_t {
	for {
		counter := ffiObject.callCounter.Load()
		if counter <= -1 {
			panic(fmt.Errorf("%v object has already been destroyed", debugName))
		}
		if counter == math.MaxInt64 {
			panic(fmt.Errorf("%v object call counter would overflow", debugName))
		}
		if ffiObject.callCounter.CompareAndSwap(counter, counter+1) {
			break
		}
	}

	return rustCall(func(status *C.RustCallStatus) C.uint64_t {
		return ffiObject.cloneFunction(ffiObject.handle, status)
	})
}

func (ffiObject *FfiObject) decrementPointer() {
	if ffiObject.callCounter.Add(-1) == -1 {
		ffiObject.freeRustArcPtr()
	}
}

func (ffiObject *FfiObject) destroy() {
	if ffiObject.destroyed.CompareAndSwap(false, true) {
		if ffiObject.callCounter.Add(-1) == -1 {
			ffiObject.freeRustArcPtr()
		}
	}
}

func (ffiObject *FfiObject) freeRustArcPtr() {
	if ffiObject.handle == 0 {
		return
	}
	rustCall(func(status *C.RustCallStatus) int32 {
		ffiObject.freeFunction(ffiObject.handle, status)
		return 0
	})
}

// Answers the broker's authentication challenges during an MQTT 5.0
// enhanced-authentication handshake (see [`ConnectOptions::auth_method`]).
type MqttAuthHandler interface {
	// The broker sent `challenge` for `method`; return the response, or
	// `None` to abort the connection attempt.
	Respond(method string, challenge []byte) *[]byte
}

// Answers the broker's authentication challenges during an MQTT 5.0
// enhanced-authentication handshake (see [`ConnectOptions::auth_method`]).
type MqttAuthHandlerImpl struct {
	ffiObject FfiObject
}

// The broker sent `challenge` for `method`; return the response, or
// `None` to abort the connection attempt.
func (_self *MqttAuthHandlerImpl) Respond(method string, challenge []byte) *[]byte {
	_pointer := _self.ffiObject.incrementPointer("MqttAuthHandler")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalBytesINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_mqtt_client_fn_method_mqttauthhandler_respond(
				_pointer, FfiConverterStringINSTANCE.Lower(method), FfiConverterBytesINSTANCE.Lower(challenge), _uniffiStatus),
		}
	}))
}
func (object *MqttAuthHandlerImpl) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttAuthHandler struct {
	handleMap *concurrentHandleMap[MqttAuthHandler]
}

var FfiConverterMqttAuthHandlerINSTANCE = FfiConverterMqttAuthHandler{
	handleMap: newConcurrentHandleMap[MqttAuthHandler](),
}

func (c FfiConverterMqttAuthHandler) Lift(handle C.uint64_t) MqttAuthHandler {
	if uint64(handle)&1 == 0 {
		// Rust-generated handle (even), construct a new object wrapping the handle
		result := &MqttAuthHandlerImpl{
			newFfiObject(
				handle,
				func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
					return C.uniffi_mqtt_client_fn_clone_mqttauthhandler(handle, status)
				},
				func(handle C.uint64_t, status *C.RustCallStatus) {
					C.uniffi_mqtt_client_fn_free_mqttauthhandler(handle, status)
				},
			),
		}
		runtime.SetFinalizer(result, (*MqttAuthHandlerImpl).Destroy)
		return result
	} else {
		// Go-generated handle (odd), retrieve from the handle map
		val, ok := c.handleMap.tryGet(uint64(handle))
		if !ok {
			panic(fmt.Errorf("no callback in handle map: %d", handle))
		}
		c.handleMap.remove(uint64(handle))
		return val
	}
}

func (c FfiConverterMqttAuthHandler) Read(reader io.Reader) MqttAuthHandler {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttAuthHandler) Lower(value MqttAuthHandler) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	if val, ok := value.(*MqttAuthHandlerImpl); ok {
		// Rust-backed object, clone the handle
		handle := val.ffiObject.incrementPointer("MqttAuthHandler")
		defer val.ffiObject.decrementPointer()
		return handle
	} else {
		// Go-backed object, insert into handle map
		return C.uint64_t(c.handleMap.insert(value))
	}
}

func (c FfiConverterMqttAuthHandler) Write(writer io.Writer, value MqttAuthHandler) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttAuthHandler(handle uint64) MqttAuthHandler {
	return FfiConverterMqttAuthHandlerINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttAuthHandler(value MqttAuthHandler) uint64 {
	return uint64(FfiConverterMqttAuthHandlerINSTANCE.Lower(value))
}

type FfiDestroyerMqttAuthHandler struct{}

func (_ FfiDestroyerMqttAuthHandler) Destroy(value MqttAuthHandler) {
	if val, ok := value.(*MqttAuthHandlerImpl); ok {
		val.Destroy()
	}
}

type uniffiCallbackResult C.int8_t

const (
	uniffiIdxCallbackFree               uniffiCallbackResult = 0
	uniffiCallbackResultSuccess         uniffiCallbackResult = 0
	uniffiCallbackResultError           uniffiCallbackResult = 1
	uniffiCallbackUnexpectedResultError uniffiCallbackResult = 2
	uniffiCallbackCancelled             uniffiCallbackResult = 3
)

type concurrentHandleMap[T any] struct {
	handles       map[uint64]T
	currentHandle uint64
	lock          sync.RWMutex
}

func newConcurrentHandleMap[T any]() *concurrentHandleMap[T] {
	return &concurrentHandleMap[T]{
		handles:       map[uint64]T{},
		currentHandle: 1,
	}
}

func (cm *concurrentHandleMap[T]) insert(obj T) uint64 {
	cm.lock.Lock()
	defer cm.lock.Unlock()

	handle := cm.currentHandle
	cm.currentHandle = cm.currentHandle + 2
	cm.handles[handle] = obj
	return handle
}

func (cm *concurrentHandleMap[T]) remove(handle uint64) {
	cm.lock.Lock()
	defer cm.lock.Unlock()

	delete(cm.handles, handle)
}

func (cm *concurrentHandleMap[T]) tryGet(handle uint64) (T, bool) {
	cm.lock.RLock()
	defer cm.lock.RUnlock()

	val, ok := cm.handles[handle]
	return val, ok
}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerMethod0
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerMethod0(uniffiHandle C.uint64_t, method C.RustBuffer, challenge C.RustBuffer, uniffiOutReturn *C.RustBuffer, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttAuthHandlerINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	res :=
		uniffiObj.Respond(
			FfiConverterStringINSTANCE.Lift(GoRustBuffer{
				inner: method,
			}),
			FfiConverterBytesINSTANCE.Lift(GoRustBuffer{
				inner: challenge,
			}),
		)

	*uniffiOutReturn = FfiConverterOptionalBytesINSTANCE.Lower(res)
}

var UniffiVTableCallbackInterfaceMqttAuthHandlerINSTANCE = C.UniffiVTableCallbackInterfaceMqttAuthHandler{
	uniffiFree:  (C.UniffiCallbackInterfaceFree)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerFree),
	uniffiClone: (C.UniffiCallbackInterfaceClone)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerClone),
	respond:     (C.UniffiCallbackInterfaceMqttAuthHandlerMethod0)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerMethod0),
}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerFree
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerFree(handle C.uint64_t) {
	FfiConverterMqttAuthHandlerINSTANCE.handleMap.remove(uint64(handle))
}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerClone
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttAuthHandlerClone(handle C.uint64_t) C.uint64_t {
	val, ok := FfiConverterMqttAuthHandlerINSTANCE.handleMap.tryGet(uint64(handle))
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return C.uint64_t(FfiConverterMqttAuthHandlerINSTANCE.handleMap.insert(val))
}

func (c FfiConverterMqttAuthHandler) register() {
	C.uniffi_mqtt_client_fn_init_callback_vtable_mqttauthhandler(&UniffiVTableCallbackInterfaceMqttAuthHandlerINSTANCE)
}

// A connected (or not-yet-connected) MQTT client instance.
//
// One `MqttClient` corresponds to one broker connection / MQTT session.
// Create it with [`MqttClient::new`], call [`MqttClient::connect`], then
// use [`MqttClient::publish`] / [`MqttClient::subscribe`] /
// [`MqttClient::unsubscribe`]. Call [`MqttClient::disconnect`] (or simply
// drop it) to close the connection.
type MqttClientInterface interface {
	// Open the TCP connection and complete the CONNECT/CONNACK handshake.
	// Safe to call again after a connection loss (including one this
	// client is still in the middle of auto-reconnecting from).
	//
	// If [`ConnectOptions::auto_reconnect`] is set, a background
	// supervisor is started (once) that automatically re-runs this same
	// handshake — with exponential backoff — after any *unexpected*
	// connection loss (i.e. not caused by [`Self::disconnect`]), then
	// replays every topic this client is currently subscribed to.
	Connect() (ConnectResult, error)
	// Gracefully disconnect: sends a DISCONNECT packet, stops background
	// tasks, and closes the socket. Safe to call multiple times. Also
	// stops the [`ConnectOptions::auto_reconnect`] supervisor, if one is
	// running — this is a deliberate disconnect, not a connection loss to
	// recover from.
	Disconnect() error
	// Pull-style alternative to [`set_message_listener`](Self::set_message_listener)
	// for runtimes that can't receive callbacks from Rust threads (Dart,
	// Haskell): keep up to `capacity` incoming messages for
	// [`next_message`](Self::next_message). `0` turns the queue off. When it
	// fills up the oldest message is dropped. May be combined with a listener.
	EnableMessageQueue(capacity uint32)
	// `true` once [`connect`](Self::connect) has succeeded and the
	// connection has not since been lost or closed.
	IsConnected() bool
	// Why the most recent connection was lost (`None` if it never was) —
	// the pull-style twin of `MqttMessageListener::on_disconnected`.
	LastDisconnectReason() *string
	// The next queued incoming message, waiting up to `timeout_ms`
	// milliseconds for one; `None` if none arrived. Requires
	// [`enable_message_queue`](Self::enable_message_queue).
	NextMessage(timeoutMs uint32) *MqttMessage
	// Publish `payload` to `topic`. Resolves once the broker has
	// acknowledged the message (QoS 1: PUBACK; QoS 2: full four-part
	// handshake). Resolves immediately for QoS 0.
	//
	// For QoS 1/2, an unacked step is retried (with DUP=1) up to
	// `MAX_PUBLISH_RETRIES` (3) times before giving up with
	// [`MqttError::Timeout`] — the broker never resends on our behalf, so
	// enforcing "at least once"/"exactly once" delivery is the
	// publisher's job.
	//
	// A topic containing wildcards (`+`, `#`) or that is empty is
	// rejected locally with [`MqttError::Protocol`] (a broker would drop
	// the whole connection for it). An MQTT 5.0 broker that answers with a
	// failure reason code (e.g. "not authorized") surfaces as
	// [`MqttError::Session`] instead of being reported as success.
	Publish(topic string, payload []byte, qos QoS, retain bool) error
	// Register the handler that answers the broker's authentication
	// challenges when [`ConnectOptions::auth_method`] is set (MQTT 5.0
	// enhanced authentication). Must be set before `connect()`.
	SetAuthHandler(handler MqttAuthHandler)
	// Register (or replace) the listener that receives incoming messages
	// and disconnect notifications. Safe to call before or after connect.
	SetMessageListener(listener MqttMessageListener)
	// Subscribe to `topic_filter` requesting at most `qos`. Returns the
	// reason/return code the broker granted.
	//
	// Recorded in this client's own subscription bookkeeping so
	// [`ConnectOptions::auto_reconnect`] can replay it after a reconnect
	// — the broker has no way to tell a resumed client what it was
	// subscribed to beyond what a persistent (`clean_start = false`)
	// session already covers.
	Subscribe(topicFilter string, qos QoS) (SubscribeResult, error)
	// Unsubscribe from `topic_filter`.
	Unsubscribe(topicFilter string) error
}

// A connected (or not-yet-connected) MQTT client instance.
//
// One `MqttClient` corresponds to one broker connection / MQTT session.
// Create it with [`MqttClient::new`], call [`MqttClient::connect`], then
// use [`MqttClient::publish`] / [`MqttClient::subscribe`] /
// [`MqttClient::unsubscribe`]. Call [`MqttClient::disconnect`] (or simply
// drop it) to close the connection.
type MqttClient struct {
	ffiObject FfiObject
}

// Create a new client from `options`. Does not connect yet.
func NewMqttClient(options ConnectOptions) *MqttClient {
	return FfiConverterMqttClientINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_mqtt_client_fn_constructor_mqttclient_new(FfiConverterConnectOptionsINSTANCE.Lower(options), _uniffiStatus)
	}))
}

// Open the TCP connection and complete the CONNECT/CONNACK handshake.
// Safe to call again after a connection loss (including one this
// client is still in the middle of auto-reconnecting from).
//
// If [`ConnectOptions::auto_reconnect`] is set, a background
// supervisor is started (once) that automatically re-runs this same
// handshake — with exponential backoff — after any *unexpected*
// connection loss (i.e. not caused by [`Self::disconnect`]), then
// replays every topic this client is currently subscribed to.
func (_self *MqttClient) Connect() (ConnectResult, error) {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[*MqttError](
		FfiConverterMqttErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_mqtt_client_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) ConnectResult {
			return FfiConverterConnectResultINSTANCE.Lift(ffi)
		},
		C.uniffi_mqtt_client_fn_method_mqttclient_connect(
			_pointer),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_client_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_client_rust_future_free_rust_buffer(handle)
		},
	)

	if err == nil {
		return res, nil
	}

	return res, err
}

// Gracefully disconnect: sends a DISCONNECT packet, stops background
// tasks, and closes the socket. Safe to call multiple times. Also
// stops the [`ConnectOptions::auto_reconnect`] supervisor, if one is
// running — this is a deliberate disconnect, not a connection loss to
// recover from.
func (_self *MqttClient) Disconnect() error {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[*MqttError](
		FfiConverterMqttErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_mqtt_client_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_mqtt_client_fn_method_mqttclient_disconnect(
			_pointer),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_client_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_client_rust_future_free_void(handle)
		},
	)

	if err == nil {
		return nil
	}

	return err
}

// Pull-style alternative to [`set_message_listener`](Self::set_message_listener)
// for runtimes that can't receive callbacks from Rust threads (Dart,
// Haskell): keep up to `capacity` incoming messages for
// [`next_message`](Self::next_message). `0` turns the queue off. When it
// fills up the oldest message is dropped. May be combined with a listener.
func (_self *MqttClient) EnableMessageQueue(capacity uint32) {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_client_fn_method_mqttclient_enable_message_queue(
			_pointer, FfiConverterUint32INSTANCE.Lower(capacity), _uniffiStatus)
		return false
	})
}

// `true` once [`connect`](Self::connect) has succeeded and the
// connection has not since been lost or closed.
func (_self *MqttClient) IsConnected() bool {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_mqtt_client_fn_method_mqttclient_is_connected(
			_pointer, _uniffiStatus)
	}))
}

// Why the most recent connection was lost (`None` if it never was) —
// the pull-style twin of `MqttMessageListener::on_disconnected`.
func (_self *MqttClient) LastDisconnectReason() *string {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_mqtt_client_fn_method_mqttclient_last_disconnect_reason(
				_pointer, _uniffiStatus),
		}
	}))
}

// The next queued incoming message, waiting up to `timeout_ms`
// milliseconds for one; `None` if none arrived. Requires
// [`enable_message_queue`](Self::enable_message_queue).
func (_self *MqttClient) NextMessage(timeoutMs uint32) *MqttMessage {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	res, _ := uniffiRustCallAsync[error](
		nil,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_mqtt_client_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) *MqttMessage {
			return FfiConverterOptionalMqttMessageINSTANCE.Lift(ffi)
		},
		C.uniffi_mqtt_client_fn_method_mqttclient_next_message(
			_pointer, FfiConverterUint32INSTANCE.Lower(timeoutMs)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_client_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_client_rust_future_free_rust_buffer(handle)
		},
	)

	return res
}

// Publish `payload` to `topic`. Resolves once the broker has
// acknowledged the message (QoS 1: PUBACK; QoS 2: full four-part
// handshake). Resolves immediately for QoS 0.
//
// For QoS 1/2, an unacked step is retried (with DUP=1) up to
// `MAX_PUBLISH_RETRIES` (3) times before giving up with
// [`MqttError::Timeout`] — the broker never resends on our behalf, so
// enforcing "at least once"/"exactly once" delivery is the
// publisher's job.
//
// A topic containing wildcards (`+`, `#`) or that is empty is
// rejected locally with [`MqttError::Protocol`] (a broker would drop
// the whole connection for it). An MQTT 5.0 broker that answers with a
// failure reason code (e.g. "not authorized") surfaces as
// [`MqttError::Session`] instead of being reported as success.
func (_self *MqttClient) Publish(topic string, payload []byte, qos QoS, retain bool) error {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[*MqttError](
		FfiConverterMqttErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_mqtt_client_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_mqtt_client_fn_method_mqttclient_publish(
			_pointer, FfiConverterStringINSTANCE.Lower(topic), FfiConverterBytesINSTANCE.Lower(payload), FfiConverterQoSINSTANCE.Lower(qos), FfiConverterBoolINSTANCE.Lower(retain)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_client_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_client_rust_future_free_void(handle)
		},
	)

	if err == nil {
		return nil
	}

	return err
}

// Register the handler that answers the broker's authentication
// challenges when [`ConnectOptions::auth_method`] is set (MQTT 5.0
// enhanced authentication). Must be set before `connect()`.
func (_self *MqttClient) SetAuthHandler(handler MqttAuthHandler) {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_client_fn_method_mqttclient_set_auth_handler(
			_pointer, FfiConverterMqttAuthHandlerINSTANCE.Lower(handler), _uniffiStatus)
		return false
	})
}

// Register (or replace) the listener that receives incoming messages
// and disconnect notifications. Safe to call before or after connect.
func (_self *MqttClient) SetMessageListener(listener MqttMessageListener) {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_client_fn_method_mqttclient_set_message_listener(
			_pointer, FfiConverterMqttMessageListenerINSTANCE.Lower(listener), _uniffiStatus)
		return false
	})
}

// Subscribe to `topic_filter` requesting at most `qos`. Returns the
// reason/return code the broker granted.
//
// Recorded in this client's own subscription bookkeeping so
// [`ConnectOptions::auto_reconnect`] can replay it after a reconnect
// — the broker has no way to tell a resumed client what it was
// subscribed to beyond what a persistent (`clean_start = false`)
// session already covers.
func (_self *MqttClient) Subscribe(topicFilter string, qos QoS) (SubscribeResult, error) {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[*MqttError](
		FfiConverterMqttErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_mqtt_client_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) SubscribeResult {
			return FfiConverterSubscribeResultINSTANCE.Lift(ffi)
		},
		C.uniffi_mqtt_client_fn_method_mqttclient_subscribe(
			_pointer, FfiConverterStringINSTANCE.Lower(topicFilter), FfiConverterQoSINSTANCE.Lower(qos)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_client_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_client_rust_future_free_rust_buffer(handle)
		},
	)

	if err == nil {
		return res, nil
	}

	return res, err
}

// Unsubscribe from `topic_filter`.
func (_self *MqttClient) Unsubscribe(topicFilter string) error {
	_pointer := _self.ffiObject.incrementPointer("*MqttClient")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[*MqttError](
		FfiConverterMqttErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_mqtt_client_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_mqtt_client_fn_method_mqttclient_unsubscribe(
			_pointer, FfiConverterStringINSTANCE.Lower(topicFilter)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_client_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_client_rust_future_free_void(handle)
		},
	)

	if err == nil {
		return nil
	}

	return err
}
func (object *MqttClient) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttClient struct{}

var FfiConverterMqttClientINSTANCE = FfiConverterMqttClient{}

func (c FfiConverterMqttClient) Lift(handle C.uint64_t) *MqttClient {
	result := &MqttClient{
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_mqtt_client_fn_clone_mqttclient(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_mqtt_client_fn_free_mqttclient(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*MqttClient).Destroy)
	return result
}

func (c FfiConverterMqttClient) Read(reader io.Reader) *MqttClient {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttClient) Lower(value *MqttClient) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*MqttClient")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterMqttClient) Write(writer io.Writer, value *MqttClient) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttClient(handle uint64) *MqttClient {
	return FfiConverterMqttClientINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttClient(value *MqttClient) uint64 {
	return uint64(FfiConverterMqttClientINSTANCE.Lower(value))
}

type FfiDestroyerMqttClient struct{}

func (_ FfiDestroyerMqttClient) Destroy(value *MqttClient) {
	value.Destroy()
}

// Callback interface implemented by foreign code to receive events from an
// [`crate::client::MqttClient`] without polling.
type MqttMessageListener interface {
	// Called for every PUBLISH delivered to a subscription this client holds.
	OnMessage(message MqttMessage)
	// Called when the connection to the broker is lost or closed, whether
	// by the client, the broker, or the network.
	OnDisconnected(reason string)
}

// Callback interface implemented by foreign code to receive events from an
// [`crate::client::MqttClient`] without polling.
type MqttMessageListenerImpl struct {
	ffiObject FfiObject
}

// Called for every PUBLISH delivered to a subscription this client holds.
func (_self *MqttMessageListenerImpl) OnMessage(message MqttMessage) {
	_pointer := _self.ffiObject.incrementPointer("MqttMessageListener")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_client_fn_method_mqttmessagelistener_on_message(
			_pointer, FfiConverterMqttMessageINSTANCE.Lower(message), _uniffiStatus)
		return false
	})
}

// Called when the connection to the broker is lost or closed, whether
// by the client, the broker, or the network.
func (_self *MqttMessageListenerImpl) OnDisconnected(reason string) {
	_pointer := _self.ffiObject.incrementPointer("MqttMessageListener")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_client_fn_method_mqttmessagelistener_on_disconnected(
			_pointer, FfiConverterStringINSTANCE.Lower(reason), _uniffiStatus)
		return false
	})
}
func (object *MqttMessageListenerImpl) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttMessageListener struct {
	handleMap *concurrentHandleMap[MqttMessageListener]
}

var FfiConverterMqttMessageListenerINSTANCE = FfiConverterMqttMessageListener{
	handleMap: newConcurrentHandleMap[MqttMessageListener](),
}

func (c FfiConverterMqttMessageListener) Lift(handle C.uint64_t) MqttMessageListener {
	if uint64(handle)&1 == 0 {
		// Rust-generated handle (even), construct a new object wrapping the handle
		result := &MqttMessageListenerImpl{
			newFfiObject(
				handle,
				func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
					return C.uniffi_mqtt_client_fn_clone_mqttmessagelistener(handle, status)
				},
				func(handle C.uint64_t, status *C.RustCallStatus) {
					C.uniffi_mqtt_client_fn_free_mqttmessagelistener(handle, status)
				},
			),
		}
		runtime.SetFinalizer(result, (*MqttMessageListenerImpl).Destroy)
		return result
	} else {
		// Go-generated handle (odd), retrieve from the handle map
		val, ok := c.handleMap.tryGet(uint64(handle))
		if !ok {
			panic(fmt.Errorf("no callback in handle map: %d", handle))
		}
		c.handleMap.remove(uint64(handle))
		return val
	}
}

func (c FfiConverterMqttMessageListener) Read(reader io.Reader) MqttMessageListener {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttMessageListener) Lower(value MqttMessageListener) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	if val, ok := value.(*MqttMessageListenerImpl); ok {
		// Rust-backed object, clone the handle
		handle := val.ffiObject.incrementPointer("MqttMessageListener")
		defer val.ffiObject.decrementPointer()
		return handle
	} else {
		// Go-backed object, insert into handle map
		return C.uint64_t(c.handleMap.insert(value))
	}
}

func (c FfiConverterMqttMessageListener) Write(writer io.Writer, value MqttMessageListener) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttMessageListener(handle uint64) MqttMessageListener {
	return FfiConverterMqttMessageListenerINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttMessageListener(value MqttMessageListener) uint64 {
	return uint64(FfiConverterMqttMessageListenerINSTANCE.Lower(value))
}

type FfiDestroyerMqttMessageListener struct{}

func (_ FfiDestroyerMqttMessageListener) Destroy(value MqttMessageListener) {
	if val, ok := value.(*MqttMessageListenerImpl); ok {
		val.Destroy()
	}
}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerMethod0
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerMethod0(uniffiHandle C.uint64_t, message C.RustBuffer, uniffiOutReturn *C.void, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttMessageListenerINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	uniffiObj.OnMessage(
		FfiConverterMqttMessageINSTANCE.Lift(GoRustBuffer{
			inner: message,
		}),
	)

}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerMethod1
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerMethod1(uniffiHandle C.uint64_t, reason C.RustBuffer, uniffiOutReturn *C.void, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttMessageListenerINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	uniffiObj.OnDisconnected(
		FfiConverterStringINSTANCE.Lift(GoRustBuffer{
			inner: reason,
		}),
	)

}

var UniffiVTableCallbackInterfaceMqttMessageListenerINSTANCE = C.UniffiVTableCallbackInterfaceMqttMessageListener{
	uniffiFree:     (C.UniffiCallbackInterfaceFree)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerFree),
	uniffiClone:    (C.UniffiCallbackInterfaceClone)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerClone),
	onMessage:      (C.UniffiCallbackInterfaceMqttMessageListenerMethod0)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerMethod0),
	onDisconnected: (C.UniffiCallbackInterfaceMqttMessageListenerMethod1)(C.mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerMethod1),
}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerFree
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerFree(handle C.uint64_t) {
	FfiConverterMqttMessageListenerINSTANCE.handleMap.remove(uint64(handle))
}

//export mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerClone
func mqtt_client_client_types_cgo_dispatchCallbackInterfaceMqttMessageListenerClone(handle C.uint64_t) C.uint64_t {
	val, ok := FfiConverterMqttMessageListenerINSTANCE.handleMap.tryGet(uint64(handle))
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return C.uint64_t(FfiConverterMqttMessageListenerINSTANCE.handleMap.insert(val))
}

func (c FfiConverterMqttMessageListener) register() {
	C.uniffi_mqtt_client_fn_init_callback_vtable_mqttmessagelistener(&UniffiVTableCallbackInterfaceMqttMessageListenerINSTANCE)
}

// Everything needed to establish an MQTT connection.
type ConnectOptions struct {
	Host     string
	Port     uint16
	ClientId string
	// Which protocol version to speak. The broker never up/downgrades a
	// connection once CONNECT is sent.
	Version MqttVersion
	// MQTT 3.1.1 "Clean Session" / MQTT 5.0 "Clean Start".
	CleanStart bool
	// Keep-alive interval in seconds. `0` disables keep-alive pings.
	KeepAliveSecs uint16
	Username      *string
	Password      *[]byte
	Will          *WillOptions
	// Seconds to wait for the TCP connection + CONNACK before failing.
	ConnectTimeoutSecs uint32
	// Seconds to wait for a QoS 1/2 handshake step *per attempt* before
	// retrying (with DUP=1) — `MqttClient::publish` retries an unacked
	// QoS 1/2 PUBLISH (or QoS 2's PUBREL) up to 3 times before giving up,
	// so the worst-case total wait for a single `publish()` call is up to
	// 4x this value. `0` selects the built-in default (15s).
	OperationTimeoutSecs uint32
	// If true, [`crate::client::MqttClient`] automatically reconnects
	// (with exponential backoff, see [`Self::reconnect_backoff_secs`] /
	// [`Self::reconnect_max_backoff_secs`]) after a connection loss that
	// wasn't caused by calling `disconnect()`, replaying every topic this
	// client is currently subscribed to (per its own bookkeeping) once
	// reconnected. Off by default — the caller must opt in.
	AutoReconnect bool
	// Initial delay before the first reconnect attempt, doubling after
	// each failed attempt up to [`Self::reconnect_max_backoff_secs`].
	// `0` selects the built-in default (1s). Ignored unless
	// [`Self::auto_reconnect`] is set.
	ReconnectBackoffSecs uint32
	// Cap on the exponential reconnect backoff delay. `0` selects the
	// built-in default (30s). Ignored unless [`Self::auto_reconnect`] is
	// set.
	ReconnectMaxBackoffSecs uint32
	// If set, connect over TLS (`mqtts`) instead of plain TCP. `None`
	// (the default) is plain TCP, unchanged from before TLS support
	// existed.
	Tls *TlsOptions
	// Largest packet (fixed header + body, in bytes) this client will
	// accept from the broker; a larger one is treated as a protocol error
	// and drops the connection instead of being buffered. `0` selects the
	// built-in default (the MQTT protocol maximum, ~256 MiB).
	MaxPacketSize uint32
	// MQTT 5.0 enhanced authentication (SCRAM, Kerberos, OAuth, …): the
	// `Authentication Method` to announce in CONNECT. The broker may then
	// send challenges, which are answered by the
	// [`MqttAuthHandler`] registered with `MqttClient::set_auth_handler`.
	// Requires [`MqttVersion::V5`].
	AuthMethod *string
	// Initial `Authentication Data` sent with CONNECT (e.g. a SCRAM
	// client-first message). Requires [`Self::auth_method`].
	AuthData *[]byte
}

func (r *ConnectOptions) Destroy() {
	FfiDestroyerString{}.Destroy(r.Host)
	FfiDestroyerUint16{}.Destroy(r.Port)
	FfiDestroyerString{}.Destroy(r.ClientId)
	FfiDestroyerMqttVersion{}.Destroy(r.Version)
	FfiDestroyerBool{}.Destroy(r.CleanStart)
	FfiDestroyerUint16{}.Destroy(r.KeepAliveSecs)
	FfiDestroyerOptionalString{}.Destroy(r.Username)
	FfiDestroyerOptionalBytes{}.Destroy(r.Password)
	FfiDestroyerOptionalWillOptions{}.Destroy(r.Will)
	FfiDestroyerUint32{}.Destroy(r.ConnectTimeoutSecs)
	FfiDestroyerUint32{}.Destroy(r.OperationTimeoutSecs)
	FfiDestroyerBool{}.Destroy(r.AutoReconnect)
	FfiDestroyerUint32{}.Destroy(r.ReconnectBackoffSecs)
	FfiDestroyerUint32{}.Destroy(r.ReconnectMaxBackoffSecs)
	FfiDestroyerOptionalTlsOptions{}.Destroy(r.Tls)
	FfiDestroyerUint32{}.Destroy(r.MaxPacketSize)
	FfiDestroyerOptionalString{}.Destroy(r.AuthMethod)
	FfiDestroyerOptionalBytes{}.Destroy(r.AuthData)
}

type FfiConverterConnectOptions struct{}

var FfiConverterConnectOptionsINSTANCE = FfiConverterConnectOptions{}

func (c FfiConverterConnectOptions) Lift(rb RustBufferI) ConnectOptions {
	return LiftFromRustBuffer[ConnectOptions](c, rb)
}

func (c FfiConverterConnectOptions) Read(reader io.Reader) ConnectOptions {
	return ConnectOptions{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterUint16INSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterMqttVersionINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterUint16INSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalBytesINSTANCE.Read(reader),
		FfiConverterOptionalWillOptionsINSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterOptionalTlsOptionsINSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalBytesINSTANCE.Read(reader),
	}
}

func (c FfiConverterConnectOptions) Lower(value ConnectOptions) C.RustBuffer {
	return LowerIntoRustBuffer[ConnectOptions](c, value)
}

func (c FfiConverterConnectOptions) LowerExternal(value ConnectOptions) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[ConnectOptions](c, value))
}

func (c FfiConverterConnectOptions) Write(writer io.Writer, value ConnectOptions) {
	FfiConverterStringINSTANCE.Write(writer, value.Host)
	FfiConverterUint16INSTANCE.Write(writer, value.Port)
	FfiConverterStringINSTANCE.Write(writer, value.ClientId)
	FfiConverterMqttVersionINSTANCE.Write(writer, value.Version)
	FfiConverterBoolINSTANCE.Write(writer, value.CleanStart)
	FfiConverterUint16INSTANCE.Write(writer, value.KeepAliveSecs)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Username)
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.Password)
	FfiConverterOptionalWillOptionsINSTANCE.Write(writer, value.Will)
	FfiConverterUint32INSTANCE.Write(writer, value.ConnectTimeoutSecs)
	FfiConverterUint32INSTANCE.Write(writer, value.OperationTimeoutSecs)
	FfiConverterBoolINSTANCE.Write(writer, value.AutoReconnect)
	FfiConverterUint32INSTANCE.Write(writer, value.ReconnectBackoffSecs)
	FfiConverterUint32INSTANCE.Write(writer, value.ReconnectMaxBackoffSecs)
	FfiConverterOptionalTlsOptionsINSTANCE.Write(writer, value.Tls)
	FfiConverterUint32INSTANCE.Write(writer, value.MaxPacketSize)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.AuthMethod)
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.AuthData)
}

type FfiDestroyerConnectOptions struct{}

func (_ FfiDestroyerConnectOptions) Destroy(value ConnectOptions) {
	value.Destroy()
}

// Outcome of a successful [`crate::client::MqttClient::connect`].
type ConnectResult struct {
	SessionPresent bool
	// `0x00` on success. MQTT 5.0 reason code (MQTT 3.1.1 codes are mapped
	// onto the equivalent 5.0 code by
	// [`ConnectReasonCode`](crate::protocol::connect::ConnectReasonCode)).
	ReasonCode uint8
}

func (r *ConnectResult) Destroy() {
	FfiDestroyerBool{}.Destroy(r.SessionPresent)
	FfiDestroyerUint8{}.Destroy(r.ReasonCode)
}

type FfiConverterConnectResult struct{}

var FfiConverterConnectResultINSTANCE = FfiConverterConnectResult{}

func (c FfiConverterConnectResult) Lift(rb RustBufferI) ConnectResult {
	return LiftFromRustBuffer[ConnectResult](c, rb)
}

func (c FfiConverterConnectResult) Read(reader io.Reader) ConnectResult {
	return ConnectResult{
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterUint8INSTANCE.Read(reader),
	}
}

func (c FfiConverterConnectResult) Lower(value ConnectResult) C.RustBuffer {
	return LowerIntoRustBuffer[ConnectResult](c, value)
}

func (c FfiConverterConnectResult) LowerExternal(value ConnectResult) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[ConnectResult](c, value))
}

func (c FfiConverterConnectResult) Write(writer io.Writer, value ConnectResult) {
	FfiConverterBoolINSTANCE.Write(writer, value.SessionPresent)
	FfiConverterUint8INSTANCE.Write(writer, value.ReasonCode)
}

type FfiDestroyerConnectResult struct{}

func (_ FfiDestroyerConnectResult) Destroy(value ConnectResult) {
	value.Destroy()
}

// A message delivered to a subscriber.
type MqttMessage struct {
	Topic   string
	Payload []byte
	Qos     QoS
	Retain  bool
}

func (r *MqttMessage) Destroy() {
	FfiDestroyerString{}.Destroy(r.Topic)
	FfiDestroyerBytes{}.Destroy(r.Payload)
	FfiDestroyerQoS{}.Destroy(r.Qos)
	FfiDestroyerBool{}.Destroy(r.Retain)
}

type FfiConverterMqttMessage struct{}

var FfiConverterMqttMessageINSTANCE = FfiConverterMqttMessage{}

func (c FfiConverterMqttMessage) Lift(rb RustBufferI) MqttMessage {
	return LiftFromRustBuffer[MqttMessage](c, rb)
}

func (c FfiConverterMqttMessage) Read(reader io.Reader) MqttMessage {
	return MqttMessage{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
		FfiConverterQoSINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterMqttMessage) Lower(value MqttMessage) C.RustBuffer {
	return LowerIntoRustBuffer[MqttMessage](c, value)
}

func (c FfiConverterMqttMessage) LowerExternal(value MqttMessage) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[MqttMessage](c, value))
}

func (c FfiConverterMqttMessage) Write(writer io.Writer, value MqttMessage) {
	FfiConverterStringINSTANCE.Write(writer, value.Topic)
	FfiConverterBytesINSTANCE.Write(writer, value.Payload)
	FfiConverterQoSINSTANCE.Write(writer, value.Qos)
	FfiConverterBoolINSTANCE.Write(writer, value.Retain)
}

type FfiDestroyerMqttMessage struct{}

func (_ FfiDestroyerMqttMessage) Destroy(value MqttMessage) {
	value.Destroy()
}

// Result of a subscribe request: the reason/return code the broker
// granted for the requested filter (`< 0x80` is success; MQTT 3.1.1 codes
// 0/1/2 map onto the granted QoS).
type SubscribeResult struct {
	ReasonCode uint8
}

func (r *SubscribeResult) Destroy() {
	FfiDestroyerUint8{}.Destroy(r.ReasonCode)
}

type FfiConverterSubscribeResult struct{}

var FfiConverterSubscribeResultINSTANCE = FfiConverterSubscribeResult{}

func (c FfiConverterSubscribeResult) Lift(rb RustBufferI) SubscribeResult {
	return LiftFromRustBuffer[SubscribeResult](c, rb)
}

func (c FfiConverterSubscribeResult) Read(reader io.Reader) SubscribeResult {
	return SubscribeResult{
		FfiConverterUint8INSTANCE.Read(reader),
	}
}

func (c FfiConverterSubscribeResult) Lower(value SubscribeResult) C.RustBuffer {
	return LowerIntoRustBuffer[SubscribeResult](c, value)
}

func (c FfiConverterSubscribeResult) LowerExternal(value SubscribeResult) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[SubscribeResult](c, value))
}

func (c FfiConverterSubscribeResult) Write(writer io.Writer, value SubscribeResult) {
	FfiConverterUint8INSTANCE.Write(writer, value.ReasonCode)
}

type FfiDestroyerSubscribeResult struct{}

func (_ FfiDestroyerSubscribeResult) Destroy(value SubscribeResult) {
	value.Destroy()
}

// TLS configuration for [`ConnectOptions::tls`]. All certificate/key
// material is PEM-encoded bytes (not file paths) so this works
// identically across every language binding without assuming a
// filesystem layout.
type TlsOptions struct {
	// Additional PEM-encoded root CA certificate(s) to trust, on top of
	// the bundled Mozilla root store (`webpki-roots`). Needed to connect
	// to a broker with a self-signed or private-CA certificate.
	CaCertPem *[]byte
	// PEM-encoded client certificate, for mutual TLS (mTLS). Requires
	// [`Self::client_key_pem`] too.
	ClientCertPem *[]byte
	// PEM-encoded client private key, for mutual TLS. Requires
	// [`Self::client_cert_pem`] too.
	ClientKeyPem *[]byte
	// Skip server certificate verification entirely. **Dangerous** — only
	// for local development against a broker with a self-signed
	// certificate you can't otherwise easily trust (e.g. one generated
	// on the fly for a demo). Never use this in production: it makes the
	// connection trivially interceptable.
	InsecureSkipCertificateVerification bool
}

func (r *TlsOptions) Destroy() {
	FfiDestroyerOptionalBytes{}.Destroy(r.CaCertPem)
	FfiDestroyerOptionalBytes{}.Destroy(r.ClientCertPem)
	FfiDestroyerOptionalBytes{}.Destroy(r.ClientKeyPem)
	FfiDestroyerBool{}.Destroy(r.InsecureSkipCertificateVerification)
}

type FfiConverterTlsOptions struct{}

var FfiConverterTlsOptionsINSTANCE = FfiConverterTlsOptions{}

func (c FfiConverterTlsOptions) Lift(rb RustBufferI) TlsOptions {
	return LiftFromRustBuffer[TlsOptions](c, rb)
}

func (c FfiConverterTlsOptions) Read(reader io.Reader) TlsOptions {
	return TlsOptions{
		FfiConverterOptionalBytesINSTANCE.Read(reader),
		FfiConverterOptionalBytesINSTANCE.Read(reader),
		FfiConverterOptionalBytesINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterTlsOptions) Lower(value TlsOptions) C.RustBuffer {
	return LowerIntoRustBuffer[TlsOptions](c, value)
}

func (c FfiConverterTlsOptions) LowerExternal(value TlsOptions) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[TlsOptions](c, value))
}

func (c FfiConverterTlsOptions) Write(writer io.Writer, value TlsOptions) {
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.CaCertPem)
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.ClientCertPem)
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.ClientKeyPem)
	FfiConverterBoolINSTANCE.Write(writer, value.InsecureSkipCertificateVerification)
}

type FfiDestroyerTlsOptions struct{}

func (_ FfiDestroyerTlsOptions) Destroy(value TlsOptions) {
	value.Destroy()
}

// Last-Will-and-Testament configuration for a [`ConnectOptions`].
type WillOptions struct {
	Topic   string
	Payload []byte
	Qos     QoS
	Retain  bool
}

func (r *WillOptions) Destroy() {
	FfiDestroyerString{}.Destroy(r.Topic)
	FfiDestroyerBytes{}.Destroy(r.Payload)
	FfiDestroyerQoS{}.Destroy(r.Qos)
	FfiDestroyerBool{}.Destroy(r.Retain)
}

type FfiConverterWillOptions struct{}

var FfiConverterWillOptionsINSTANCE = FfiConverterWillOptions{}

func (c FfiConverterWillOptions) Lift(rb RustBufferI) WillOptions {
	return LiftFromRustBuffer[WillOptions](c, rb)
}

func (c FfiConverterWillOptions) Read(reader io.Reader) WillOptions {
	return WillOptions{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
		FfiConverterQoSINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterWillOptions) Lower(value WillOptions) C.RustBuffer {
	return LowerIntoRustBuffer[WillOptions](c, value)
}

func (c FfiConverterWillOptions) LowerExternal(value WillOptions) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[WillOptions](c, value))
}

func (c FfiConverterWillOptions) Write(writer io.Writer, value WillOptions) {
	FfiConverterStringINSTANCE.Write(writer, value.Topic)
	FfiConverterBytesINSTANCE.Write(writer, value.Payload)
	FfiConverterQoSINSTANCE.Write(writer, value.Qos)
	FfiConverterBoolINSTANCE.Write(writer, value.Retain)
}

type FfiDestroyerWillOptions struct{}

func (_ FfiDestroyerWillOptions) Destroy(value WillOptions) {
	value.Destroy()
}

// All errors that can be produced while encoding/decoding MQTT packets or
// operating an [`crate::client::MqttClient`].
//
// Exposed to foreign languages through UniFFI as a plain error enum.
type MqttError struct {
	err error
}

// Convenience method to turn *MqttError into error
// Avoiding treating nil pointer as non nil error interface
func (err *MqttError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err MqttError) Error() string {
	return fmt.Sprintf("MqttError: %s", err.err.Error())
}

func (err MqttError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrMqttErrorMalformedPacket = fmt.Errorf("MqttErrorMalformedPacket")
var ErrMqttErrorProtocol = fmt.Errorf("MqttErrorProtocol")
var ErrMqttErrorIo = fmt.Errorf("MqttErrorIo")
var ErrMqttErrorConnectionRefused = fmt.Errorf("MqttErrorConnectionRefused")
var ErrMqttErrorNotConnected = fmt.Errorf("MqttErrorNotConnected")
var ErrMqttErrorPacketTooLarge = fmt.Errorf("MqttErrorPacketTooLarge")
var ErrMqttErrorAlreadyConnected = fmt.Errorf("MqttErrorAlreadyConnected")
var ErrMqttErrorTimeout = fmt.Errorf("MqttErrorTimeout")
var ErrMqttErrorSession = fmt.Errorf("MqttErrorSession")

// Variant structs
// The peer sent a packet that violates the MQTT specification.
type MqttErrorMalformedPacket struct {
	message string
}

// The peer sent a packet that violates the MQTT specification.
func NewMqttErrorMalformedPacket() *MqttError {
	return &MqttError{err: &MqttErrorMalformedPacket{}}
}

func (e MqttErrorMalformedPacket) destroy() {
}

func (err MqttErrorMalformedPacket) Error() string {
	return fmt.Sprintf("MalformedPacket: %s", err.message)
}

func (self MqttErrorMalformedPacket) Is(target error) bool {
	return target == ErrMqttErrorMalformedPacket
}

// A value could not be encoded/decoded per the wire protocol.
type MqttErrorProtocol struct {
	message string
}

// A value could not be encoded/decoded per the wire protocol.
func NewMqttErrorProtocol() *MqttError {
	return &MqttError{err: &MqttErrorProtocol{}}
}

func (e MqttErrorProtocol) destroy() {
}

func (err MqttErrorProtocol) Error() string {
	return fmt.Sprintf("Protocol: %s", err.message)
}

func (self MqttErrorProtocol) Is(target error) bool {
	return target == ErrMqttErrorProtocol
}

// Underlying I/O failure (connection reset, DNS failure, etc.).
type MqttErrorIo struct {
	message string
}

// Underlying I/O failure (connection reset, DNS failure, etc.).
func NewMqttErrorIo() *MqttError {
	return &MqttError{err: &MqttErrorIo{}}
}

func (e MqttErrorIo) destroy() {
}

func (err MqttErrorIo) Error() string {
	return fmt.Sprintf("Io: %s", err.message)
}

func (self MqttErrorIo) Is(target error) bool {
	return target == ErrMqttErrorIo
}

// The broker rejected the connection attempt.
type MqttErrorConnectionRefused struct {
	message string
}

// The broker rejected the connection attempt.
func NewMqttErrorConnectionRefused() *MqttError {
	return &MqttError{err: &MqttErrorConnectionRefused{}}
}

func (e MqttErrorConnectionRefused) destroy() {
}

func (err MqttErrorConnectionRefused) Error() string {
	return fmt.Sprintf("ConnectionRefused: %s", err.message)
}

func (self MqttErrorConnectionRefused) Is(target error) bool {
	return target == ErrMqttErrorConnectionRefused
}

// An operation was attempted while not connected to a broker.
type MqttErrorNotConnected struct {
	message string
}

// An operation was attempted while not connected to a broker.
func NewMqttErrorNotConnected() *MqttError {
	return &MqttError{err: &MqttErrorNotConnected{}}
}

func (e MqttErrorNotConnected) destroy() {
}

func (err MqttErrorNotConnected) Error() string {
	return fmt.Sprintf("NotConnected: %s", err.message)
}

func (self MqttErrorNotConnected) Is(target error) bool {
	return target == ErrMqttErrorNotConnected
}

// A packet's declared size exceeds the configured maximum packet size.
type MqttErrorPacketTooLarge struct {
	message string
}

// A packet's declared size exceeds the configured maximum packet size.
func NewMqttErrorPacketTooLarge() *MqttError {
	return &MqttError{err: &MqttErrorPacketTooLarge{}}
}

func (e MqttErrorPacketTooLarge) destroy() {
}

func (err MqttErrorPacketTooLarge) Error() string {
	return fmt.Sprintf("PacketTooLarge: %s", err.message)
}

func (self MqttErrorPacketTooLarge) Is(target error) bool {
	return target == ErrMqttErrorPacketTooLarge
}

// The client is already connected.
type MqttErrorAlreadyConnected struct {
	message string
}

// The client is already connected.
func NewMqttErrorAlreadyConnected() *MqttError {
	return &MqttError{err: &MqttErrorAlreadyConnected{}}
}

func (e MqttErrorAlreadyConnected) destroy() {
}

func (err MqttErrorAlreadyConnected) Error() string {
	return fmt.Sprintf("AlreadyConnected: %s", err.message)
}

func (self MqttErrorAlreadyConnected) Is(target error) bool {
	return target == ErrMqttErrorAlreadyConnected
}

// The peer did not acknowledge a QoS 1/2 packet before the timeout.
type MqttErrorTimeout struct {
	message string
}

// The peer did not acknowledge a QoS 1/2 packet before the timeout.
func NewMqttErrorTimeout() *MqttError {
	return &MqttError{err: &MqttErrorTimeout{}}
}

func (e MqttErrorTimeout) destroy() {
}

func (err MqttErrorTimeout) Error() string {
	return fmt.Sprintf("Timeout: %s", err.message)
}

func (self MqttErrorTimeout) Is(target error) bool {
	return target == ErrMqttErrorTimeout
}

// Generic keep-alive / session error.
type MqttErrorSession struct {
	message string
}

// Generic keep-alive / session error.
func NewMqttErrorSession() *MqttError {
	return &MqttError{err: &MqttErrorSession{}}
}

func (e MqttErrorSession) destroy() {
}

func (err MqttErrorSession) Error() string {
	return fmt.Sprintf("Session: %s", err.message)
}

func (self MqttErrorSession) Is(target error) bool {
	return target == ErrMqttErrorSession
}

type FfiConverterMqttError struct{}

var FfiConverterMqttErrorINSTANCE = FfiConverterMqttError{}

func (c FfiConverterMqttError) Lift(eb RustBufferI) *MqttError {
	return LiftFromRustBuffer[*MqttError](c, eb)
}

func (c FfiConverterMqttError) Lower(value *MqttError) C.RustBuffer {
	return LowerIntoRustBuffer[*MqttError](c, value)
}

func (c FfiConverterMqttError) LowerExternal(value *MqttError) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*MqttError](c, value))
}

func (c FfiConverterMqttError) Read(reader io.Reader) *MqttError {
	errorID := readUint32(reader)

	message := FfiConverterStringINSTANCE.Read(reader)
	switch errorID {
	case 1:
		return &MqttError{&MqttErrorMalformedPacket{message}}
	case 2:
		return &MqttError{&MqttErrorProtocol{message}}
	case 3:
		return &MqttError{&MqttErrorIo{message}}
	case 4:
		return &MqttError{&MqttErrorConnectionRefused{message}}
	case 5:
		return &MqttError{&MqttErrorNotConnected{message}}
	case 6:
		return &MqttError{&MqttErrorPacketTooLarge{message}}
	case 7:
		return &MqttError{&MqttErrorAlreadyConnected{message}}
	case 8:
		return &MqttError{&MqttErrorTimeout{message}}
	case 9:
		return &MqttError{&MqttErrorSession{message}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterMqttError.Read()", errorID))
	}

}

func (c FfiConverterMqttError) Write(writer io.Writer, value *MqttError) {
	switch variantValue := value.err.(type) {
	case *MqttErrorMalformedPacket:
		writeInt32(writer, 1)
	case *MqttErrorProtocol:
		writeInt32(writer, 2)
	case *MqttErrorIo:
		writeInt32(writer, 3)
	case *MqttErrorConnectionRefused:
		writeInt32(writer, 4)
	case *MqttErrorNotConnected:
		writeInt32(writer, 5)
	case *MqttErrorPacketTooLarge:
		writeInt32(writer, 6)
	case *MqttErrorAlreadyConnected:
		writeInt32(writer, 7)
	case *MqttErrorTimeout:
		writeInt32(writer, 8)
	case *MqttErrorSession:
		writeInt32(writer, 9)
	default:
		_ = variantValue
		panic(fmt.Sprintf("invalid error value `%v` in FfiConverterMqttError.Write", value))
	}
}

type FfiDestroyerMqttError struct{}

func (_ FfiDestroyerMqttError) Destroy(value *MqttError) {
	switch variantValue := value.err.(type) {
	case MqttErrorMalformedPacket:
		variantValue.destroy()
	case MqttErrorProtocol:
		variantValue.destroy()
	case MqttErrorIo:
		variantValue.destroy()
	case MqttErrorConnectionRefused:
		variantValue.destroy()
	case MqttErrorNotConnected:
		variantValue.destroy()
	case MqttErrorPacketTooLarge:
		variantValue.destroy()
	case MqttErrorAlreadyConnected:
		variantValue.destroy()
	case MqttErrorTimeout:
		variantValue.destroy()
	case MqttErrorSession:
		variantValue.destroy()
	default:
		_ = variantValue
		panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerMqttError.Destroy", value))
	}
}

// The MQTT protocol version negotiated for a connection.
type MqttVersion uint

const (
	// MQTT 3.1.1 (protocol level 4).
	MqttVersionV311 MqttVersion = 1
	// MQTT 5.0 (protocol level 5).
	MqttVersionV5 MqttVersion = 2
)

type FfiConverterMqttVersion struct{}

var FfiConverterMqttVersionINSTANCE = FfiConverterMqttVersion{}

func (c FfiConverterMqttVersion) Lift(rb RustBufferI) MqttVersion {
	return LiftFromRustBuffer[MqttVersion](c, rb)
}

func (c FfiConverterMqttVersion) Lower(value MqttVersion) C.RustBuffer {
	return LowerIntoRustBuffer[MqttVersion](c, value)
}

func (c FfiConverterMqttVersion) LowerExternal(value MqttVersion) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[MqttVersion](c, value))
}
func (FfiConverterMqttVersion) Read(reader io.Reader) MqttVersion {
	id := readInt32(reader)
	return MqttVersion(id)
}

func (FfiConverterMqttVersion) Write(writer io.Writer, value MqttVersion) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerMqttVersion struct{}

func (_ FfiDestroyerMqttVersion) Destroy(value MqttVersion) {
}

// MQTT Quality of Service level.
type QoS uint

const (
	// QoS 0 — fire and forget, no acknowledgement.
	QoSAtMostOnce QoS = 1
	// QoS 1 — acknowledged delivery, may be duplicated.
	QoSAtLeastOnce QoS = 2
	// QoS 2 — exactly-once delivery via a four-part handshake.
	QoSExactlyOnce QoS = 3
)

type FfiConverterQoS struct{}

var FfiConverterQoSINSTANCE = FfiConverterQoS{}

func (c FfiConverterQoS) Lift(rb RustBufferI) QoS {
	return LiftFromRustBuffer[QoS](c, rb)
}

func (c FfiConverterQoS) Lower(value QoS) C.RustBuffer {
	return LowerIntoRustBuffer[QoS](c, value)
}

func (c FfiConverterQoS) LowerExternal(value QoS) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[QoS](c, value))
}
func (FfiConverterQoS) Read(reader io.Reader) QoS {
	id := readInt32(reader)
	return QoS(id)
}

func (FfiConverterQoS) Write(writer io.Writer, value QoS) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerQoS struct{}

func (_ FfiDestroyerQoS) Destroy(value QoS) {
}

type FfiConverterOptionalString struct{}

var FfiConverterOptionalStringINSTANCE = FfiConverterOptionalString{}

func (c FfiConverterOptionalString) Lift(rb RustBufferI) *string {
	return LiftFromRustBuffer[*string](c, rb)
}

func (_ FfiConverterOptionalString) Read(reader io.Reader) *string {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterStringINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalString) Lower(value *string) C.RustBuffer {
	return LowerIntoRustBuffer[*string](c, value)
}

func (c FfiConverterOptionalString) LowerExternal(value *string) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*string](c, value))
}

func (_ FfiConverterOptionalString) Write(writer io.Writer, value *string) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalString struct{}

func (_ FfiDestroyerOptionalString) Destroy(value *string) {
	if value != nil {
		FfiDestroyerString{}.Destroy(*value)
	}
}

type FfiConverterOptionalBytes struct{}

var FfiConverterOptionalBytesINSTANCE = FfiConverterOptionalBytes{}

func (c FfiConverterOptionalBytes) Lift(rb RustBufferI) *[]byte {
	return LiftFromRustBuffer[*[]byte](c, rb)
}

func (_ FfiConverterOptionalBytes) Read(reader io.Reader) *[]byte {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterBytesINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalBytes) Lower(value *[]byte) C.RustBuffer {
	return LowerIntoRustBuffer[*[]byte](c, value)
}

func (c FfiConverterOptionalBytes) LowerExternal(value *[]byte) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*[]byte](c, value))
}

func (_ FfiConverterOptionalBytes) Write(writer io.Writer, value *[]byte) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterBytesINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalBytes struct{}

func (_ FfiDestroyerOptionalBytes) Destroy(value *[]byte) {
	if value != nil {
		FfiDestroyerBytes{}.Destroy(*value)
	}
}

type FfiConverterOptionalMqttMessage struct{}

var FfiConverterOptionalMqttMessageINSTANCE = FfiConverterOptionalMqttMessage{}

func (c FfiConverterOptionalMqttMessage) Lift(rb RustBufferI) *MqttMessage {
	return LiftFromRustBuffer[*MqttMessage](c, rb)
}

func (_ FfiConverterOptionalMqttMessage) Read(reader io.Reader) *MqttMessage {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterMqttMessageINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalMqttMessage) Lower(value *MqttMessage) C.RustBuffer {
	return LowerIntoRustBuffer[*MqttMessage](c, value)
}

func (c FfiConverterOptionalMqttMessage) LowerExternal(value *MqttMessage) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*MqttMessage](c, value))
}

func (_ FfiConverterOptionalMqttMessage) Write(writer io.Writer, value *MqttMessage) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterMqttMessageINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalMqttMessage struct{}

func (_ FfiDestroyerOptionalMqttMessage) Destroy(value *MqttMessage) {
	if value != nil {
		FfiDestroyerMqttMessage{}.Destroy(*value)
	}
}

type FfiConverterOptionalTlsOptions struct{}

var FfiConverterOptionalTlsOptionsINSTANCE = FfiConverterOptionalTlsOptions{}

func (c FfiConverterOptionalTlsOptions) Lift(rb RustBufferI) *TlsOptions {
	return LiftFromRustBuffer[*TlsOptions](c, rb)
}

func (_ FfiConverterOptionalTlsOptions) Read(reader io.Reader) *TlsOptions {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterTlsOptionsINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalTlsOptions) Lower(value *TlsOptions) C.RustBuffer {
	return LowerIntoRustBuffer[*TlsOptions](c, value)
}

func (c FfiConverterOptionalTlsOptions) LowerExternal(value *TlsOptions) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*TlsOptions](c, value))
}

func (_ FfiConverterOptionalTlsOptions) Write(writer io.Writer, value *TlsOptions) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterTlsOptionsINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalTlsOptions struct{}

func (_ FfiDestroyerOptionalTlsOptions) Destroy(value *TlsOptions) {
	if value != nil {
		FfiDestroyerTlsOptions{}.Destroy(*value)
	}
}

type FfiConverterOptionalWillOptions struct{}

var FfiConverterOptionalWillOptionsINSTANCE = FfiConverterOptionalWillOptions{}

func (c FfiConverterOptionalWillOptions) Lift(rb RustBufferI) *WillOptions {
	return LiftFromRustBuffer[*WillOptions](c, rb)
}

func (_ FfiConverterOptionalWillOptions) Read(reader io.Reader) *WillOptions {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterWillOptionsINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalWillOptions) Lower(value *WillOptions) C.RustBuffer {
	return LowerIntoRustBuffer[*WillOptions](c, value)
}

func (c FfiConverterOptionalWillOptions) LowerExternal(value *WillOptions) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*WillOptions](c, value))
}

func (_ FfiConverterOptionalWillOptions) Write(writer io.Writer, value *WillOptions) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterWillOptionsINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalWillOptions struct{}

func (_ FfiDestroyerOptionalWillOptions) Destroy(value *WillOptions) {
	if value != nil {
		FfiDestroyerWillOptions{}.Destroy(*value)
	}
}

const (
	uniffiRustFuturePollReady      int8 = 0
	uniffiRustFuturePollMaybeReady int8 = 1
)

type rustFuturePollFunc func(C.uint64_t, C.UniffiRustFutureContinuationCallback, C.uint64_t)
type rustFutureCompleteFunc[T any] func(C.uint64_t, *C.RustCallStatus) T
type rustFutureFreeFunc func(C.uint64_t)

//export mqtt_client_uniffiFutureContinuationCallback
func mqtt_client_uniffiFutureContinuationCallback(data C.uint64_t, pollResult C.int8_t) {
	h := cgo.Handle(uintptr(data))
	waiter := h.Value().(chan int8)
	waiter <- int8(pollResult)
}

func uniffiRustCallAsync[E any, T any, F any](
	errConverter BufReader[E],
	completeFunc rustFutureCompleteFunc[F],
	liftFunc func(F) T,
	rustFuture C.uint64_t,
	pollFunc rustFuturePollFunc,
	freeFunc rustFutureFreeFunc,
) (T, E) {
	defer freeFunc(rustFuture)

	pollResult := int8(-1)
	waiter := make(chan int8, 1)

	chanHandle := cgo.NewHandle(waiter)
	defer chanHandle.Delete()

	for pollResult != uniffiRustFuturePollReady {
		pollFunc(
			rustFuture,
			(C.UniffiRustFutureContinuationCallback)(C.mqtt_client_uniffiFutureContinuationCallback),
			C.uint64_t(chanHandle),
		)
		pollResult = <-waiter
	}

	var goValue T
	ffiValue, err := rustCallWithError(errConverter, func(status *C.RustCallStatus) F {
		return completeFunc(rustFuture, status)
	})
	if value := reflect.ValueOf(err); value.IsValid() && !value.IsZero() {
		return goValue, err
	}
	return liftFunc(ffiValue), err
}

//export mqtt_client_uniffiFreeGorutine
func mqtt_client_uniffiFreeGorutine(data C.uint64_t) {
	handle := cgo.Handle(uintptr(data))
	defer handle.Delete()

	guard := handle.Value().(chan struct{})
	guard <- struct{}{}
}
