package mqtt_broker

// #cgo CFLAGS: -I${SRCDIR}
// #cgo LDFLAGS: -lmqtt_broker
// #include <mqtt_broker.h>
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
		C.ffi_mqtt_broker_rustbuffer_free(cb.inner, status)
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
		return C.ffi_mqtt_broker_rustbuffer_from_bytes(foreign, status)
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

	FfiConverterMqttAuthProviderINSTANCE.register()
	FfiConverterMqttBrokerEventListenerINSTANCE.register()
	FfiConverterMqttEnhancedAuthProviderINSTANCE.register()
	uniffiCheckChecksums()
}

func uniffiCheckChecksums() {
	// Get the bindings contract version from our ComponentInterface
	bindingsContractVersion := 30
	// Get the scaffolding contract version by calling the into the dylib
	scaffoldingContractVersion := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.ffi_mqtt_broker_uniffi_contract_version()
	})
	if bindingsContractVersion != int(scaffoldingContractVersion) {
		// If this happens try cleaning and rebuilding your project
		panic("mqtt_broker: UniFFI contract version mismatch")
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_bound_port()
		})
		if checksum != 22290 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_bound_port: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_bound_tls_port()
		})
		if checksum != 54584 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_bound_tls_port: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_bound_ws_port()
		})
		if checksum != 60355 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_bound_ws_port: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_client_count()
		})
		if checksum != 1099 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_client_count: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_enable_auth_queue()
		})
		if checksum != 8992 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_enable_auth_queue: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_enable_event_queue()
		})
		if checksum != 18590 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_enable_event_queue: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_is_running()
		})
		if checksum != 21275 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_is_running: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_next_auth_request()
		})
		if checksum != 35510 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_next_auth_request: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_next_event()
		})
		if checksum != 53279 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_next_event: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_resolve_auth_request()
		})
		if checksum != 57367 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_resolve_auth_request: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_set_auth_provider()
		})
		if checksum != 50368 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_set_auth_provider: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_set_enhanced_auth_provider()
		})
		if checksum != 28894 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_set_enhanced_auth_provider: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_set_event_listener()
		})
		if checksum != 34596 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_set_event_listener: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_start()
		})
		if checksum != 53462 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_start: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbroker_stop()
		})
		if checksum != 40506 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbroker_stop: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttauthprovider_authenticate()
		})
		if checksum != 4915 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttauthprovider_authenticate: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbrokereventlistener_on_client_connected()
		})
		if checksum != 17892 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbrokereventlistener_on_client_connected: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbrokereventlistener_on_client_disconnected()
		})
		if checksum != 32685 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbrokereventlistener_on_client_disconnected: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttbrokereventlistener_on_message_published()
		})
		if checksum != 22792 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttbrokereventlistener_on_message_published: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_method_mqttenhancedauthprovider_step()
		})
		if checksum != 29637 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_method_mqttenhancedauthprovider_step: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_mqtt_broker_checksum_constructor_mqttbroker_new()
		})
		if checksum != 33369 {
			// If this happens try cleaning and rebuilding your project
			panic("mqtt_broker: uniffi_mqtt_broker_checksum_constructor_mqttbroker_new: UniFFI API checksum mismatch")
		}
	}
}

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

type FfiConverterUint64 struct{}

var FfiConverterUint64INSTANCE = FfiConverterUint64{}

func (FfiConverterUint64) Lower(value uint64) C.uint64_t {
	return C.uint64_t(value)
}

func (FfiConverterUint64) Write(writer io.Writer, value uint64) {
	writeUint64(writer, value)
}

func (FfiConverterUint64) Lift(value C.uint64_t) uint64 {
	return uint64(value)
}

func (FfiConverterUint64) Read(reader io.Reader) uint64 {
	return readUint64(reader)
}

type FfiDestroyerUint64 struct{}

func (FfiDestroyerUint64) Destroy(_ uint64) {}

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

// Pluggable authentication, evaluated once per CONNECT.
type MqttAuthProvider interface {
	// Return `true` to accept the connection.
	Authenticate(clientId string, username *string, password *[]byte) bool
}

// Pluggable authentication, evaluated once per CONNECT.
type MqttAuthProviderImpl struct {
	ffiObject FfiObject
}

// Return `true` to accept the connection.
func (_self *MqttAuthProviderImpl) Authenticate(clientId string, username *string, password *[]byte) bool {
	_pointer := _self.ffiObject.incrementPointer("MqttAuthProvider")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_mqtt_broker_fn_method_mqttauthprovider_authenticate(
			_pointer, FfiConverterStringINSTANCE.Lower(clientId), FfiConverterOptionalStringINSTANCE.Lower(username), FfiConverterOptionalBytesINSTANCE.Lower(password), _uniffiStatus)
	}))
}
func (object *MqttAuthProviderImpl) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttAuthProvider struct {
	handleMap *concurrentHandleMap[MqttAuthProvider]
}

var FfiConverterMqttAuthProviderINSTANCE = FfiConverterMqttAuthProvider{
	handleMap: newConcurrentHandleMap[MqttAuthProvider](),
}

func (c FfiConverterMqttAuthProvider) Lift(handle C.uint64_t) MqttAuthProvider {
	if uint64(handle)&1 == 0 {
		// Rust-generated handle (even), construct a new object wrapping the handle
		result := &MqttAuthProviderImpl{
			newFfiObject(
				handle,
				func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
					return C.uniffi_mqtt_broker_fn_clone_mqttauthprovider(handle, status)
				},
				func(handle C.uint64_t, status *C.RustCallStatus) {
					C.uniffi_mqtt_broker_fn_free_mqttauthprovider(handle, status)
				},
			),
		}
		runtime.SetFinalizer(result, (*MqttAuthProviderImpl).Destroy)
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

func (c FfiConverterMqttAuthProvider) Read(reader io.Reader) MqttAuthProvider {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttAuthProvider) Lower(value MqttAuthProvider) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	if val, ok := value.(*MqttAuthProviderImpl); ok {
		// Rust-backed object, clone the handle
		handle := val.ffiObject.incrementPointer("MqttAuthProvider")
		defer val.ffiObject.decrementPointer()
		return handle
	} else {
		// Go-backed object, insert into handle map
		return C.uint64_t(c.handleMap.insert(value))
	}
}

func (c FfiConverterMqttAuthProvider) Write(writer io.Writer, value MqttAuthProvider) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttAuthProvider(handle uint64) MqttAuthProvider {
	return FfiConverterMqttAuthProviderINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttAuthProvider(value MqttAuthProvider) uint64 {
	return uint64(FfiConverterMqttAuthProviderINSTANCE.Lower(value))
}

type FfiDestroyerMqttAuthProvider struct{}

func (_ FfiDestroyerMqttAuthProvider) Destroy(value MqttAuthProvider) {
	if val, ok := value.(*MqttAuthProviderImpl); ok {
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

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderMethod0
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderMethod0(uniffiHandle C.uint64_t, clientId C.RustBuffer, username C.RustBuffer, password C.RustBuffer, uniffiOutReturn *C.int8_t, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttAuthProviderINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	res :=
		uniffiObj.Authenticate(
			FfiConverterStringINSTANCE.Lift(GoRustBuffer{
				inner: clientId,
			}),
			FfiConverterOptionalStringINSTANCE.Lift(GoRustBuffer{
				inner: username,
			}),
			FfiConverterOptionalBytesINSTANCE.Lift(GoRustBuffer{
				inner: password,
			}),
		)

	*uniffiOutReturn = FfiConverterBoolINSTANCE.Lower(res)
}

var UniffiVTableCallbackInterfaceMqttAuthProviderINSTANCE = C.UniffiVTableCallbackInterfaceMqttAuthProvider{
	uniffiFree:   (C.UniffiCallbackInterfaceFree)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderFree),
	uniffiClone:  (C.UniffiCallbackInterfaceClone)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderClone),
	authenticate: (C.UniffiCallbackInterfaceMqttAuthProviderMethod0)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderMethod0),
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderFree
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderFree(handle C.uint64_t) {
	FfiConverterMqttAuthProviderINSTANCE.handleMap.remove(uint64(handle))
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderClone
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttAuthProviderClone(handle C.uint64_t) C.uint64_t {
	val, ok := FfiConverterMqttAuthProviderINSTANCE.handleMap.tryGet(uint64(handle))
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return C.uint64_t(FfiConverterMqttAuthProviderINSTANCE.handleMap.insert(val))
}

func (c FfiConverterMqttAuthProvider) register() {
	C.uniffi_mqtt_broker_fn_init_callback_vtable_mqttauthprovider(&UniffiVTableCallbackInterfaceMqttAuthProviderINSTANCE)
}

// A running (or not-yet-started) MQTT broker instance, speaking both MQTT
// 3.1.1 and MQTT 5.0 — the protocol version is negotiated independently
// per connection from each client's CONNECT packet.
//
// Dropping a broker stops it (listeners are closed and live connections
// told to shut down); there is no way to reach a broker that has no
// remaining handle, so leaving it running would just leak its ports.
type MqttBrokerInterface interface {
	// The TCP port actually bound (useful when `config.port == 0` was used
	// to request an ephemeral port, e.g. in tests).
	BoundPort() *uint16
	// The TLS port actually bound, if `config.tls` is set.
	BoundTlsPort() *uint16
	// The WebSocket port actually bound, if `config.ws_port` is set.
	BoundWsPort() *uint16
	// Number of currently connected clients.
	ClientCount() uint32
	// Pull-style authentication for runtimes that can't implement
	// [`MqttAuthProvider`] (Dart, Haskell). Once enabled — and with no
	// provider set — every CONNECT is queued as an [`AuthRequest`](crate::AuthRequest)
	// for [`next_auth_request`](Self::next_auth_request); answer it with
	// [`resolve_auth_request`](Self::resolve_auth_request). A request left
	// unanswered for 10 seconds, or dropped because more than `capacity` are
	// waiting, is **denied**. `0` disables the queue (and denies anything
	// waiting); the broker then falls back to `allow_anonymous`.
	EnableAuthQueue(capacity uint32)
	// Pull-style alternative to [`set_event_listener`](Self::set_event_listener)
	// for runtimes that can't receive callbacks from Rust threads (Dart,
	// Haskell): keep up to `capacity` events for [`next_event`](Self::next_event).
	// `0` turns the queue off; when it fills up the oldest event is dropped.
	EnableEventQueue(capacity uint32)
	IsRunning() bool
	// The next CONNECT awaiting a decision, waiting up to `timeout_ms`
	// milliseconds; `None` if none arrived. Requires
	// [`enable_auth_queue`](Self::enable_auth_queue).
	NextAuthRequest(timeoutMs uint32) *AuthRequest
	// The next queued [`BrokerEvent`], waiting up to `timeout_ms`
	// milliseconds; `None` if none arrived. Requires
	// [`enable_event_queue`](Self::enable_event_queue).
	NextEvent(timeoutMs uint32) *BrokerEvent
	// Admit (`allow = true`) or refuse the CONNECT behind request `id`.
	// Returns `false` if the request is unknown or already expired.
	ResolveAuthRequest(id uint64, allow bool) bool
	// Register a pluggable authentication provider. Must be called before
	// [`start`](Self::start) to affect connections made right away, but
	// may be replaced at any time.
	SetAuthProvider(provider MqttAuthProvider)
	// Register the MQTT 5.0 enhanced-authentication provider (multi-round
	// challenge/response during CONNECT). Without one, a CONNECT that names
	// an `Authentication Method` is refused with "Bad authentication method".
	SetEnhancedAuthProvider(provider MqttEnhancedAuthProvider)
	// Register a listener for connect/disconnect/publish events.
	SetEventListener(listener MqttBrokerEventListener)
	// Bind the listening socket(s) and start accepting connections. Also
	// binds a second, MQTT-over-WebSocket listener on `config.ws_port` and
	// a TLS listener on `config.tls` when those are configured.
	//
	// All-or-nothing: every socket is bound (and the TLS configuration
	// validated) *before* anything starts accepting, so a failure — say
	// the WebSocket port is taken — leaves the broker cleanly stopped
	// instead of half-running with some listeners already live.
	Start() error
	// Stop the broker: close the listening sockets and ask every
	// connected client's connection to shut down. Sessions that persist
	// across disconnects (`clean_start = false`) are kept in memory, so a
	// later [`start`](Self::start) resumes them. Calling `stop()` on a
	// broker that isn't running is a no-op.
	Stop() error
}

// A running (or not-yet-started) MQTT broker instance, speaking both MQTT
// 3.1.1 and MQTT 5.0 — the protocol version is negotiated independently
// per connection from each client's CONNECT packet.
//
// Dropping a broker stops it (listeners are closed and live connections
// told to shut down); there is no way to reach a broker that has no
// remaining handle, so leaving it running would just leak its ports.
type MqttBroker struct {
	ffiObject FfiObject
}

func NewMqttBroker(config MqttBrokerConfig) *MqttBroker {
	return FfiConverterMqttBrokerINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_mqtt_broker_fn_constructor_mqttbroker_new(FfiConverterMqttBrokerConfigINSTANCE.Lower(config), _uniffiStatus)
	}))
}

// The TCP port actually bound (useful when `config.port == 0` was used
// to request an ephemeral port, e.g. in tests).
func (_self *MqttBroker) BoundPort() *uint16 {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalUint16INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_mqtt_broker_fn_method_mqttbroker_bound_port(
				_pointer, _uniffiStatus),
		}
	}))
}

// The TLS port actually bound, if `config.tls` is set.
func (_self *MqttBroker) BoundTlsPort() *uint16 {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalUint16INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_mqtt_broker_fn_method_mqttbroker_bound_tls_port(
				_pointer, _uniffiStatus),
		}
	}))
}

// The WebSocket port actually bound, if `config.ws_port` is set.
func (_self *MqttBroker) BoundWsPort() *uint16 {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalUint16INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_mqtt_broker_fn_method_mqttbroker_bound_ws_port(
				_pointer, _uniffiStatus),
		}
	}))
}

// Number of currently connected clients.
func (_self *MqttBroker) ClientCount() uint32 {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterUint32INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.uniffi_mqtt_broker_fn_method_mqttbroker_client_count(
			_pointer, _uniffiStatus)
	}))
}

// Pull-style authentication for runtimes that can't implement
// [`MqttAuthProvider`] (Dart, Haskell). Once enabled — and with no
// provider set — every CONNECT is queued as an [`AuthRequest`](crate::AuthRequest)
// for [`next_auth_request`](Self::next_auth_request); answer it with
// [`resolve_auth_request`](Self::resolve_auth_request). A request left
// unanswered for 10 seconds, or dropped because more than `capacity` are
// waiting, is **denied**. `0` disables the queue (and denies anything
// waiting); the broker then falls back to `allow_anonymous`.
func (_self *MqttBroker) EnableAuthQueue(capacity uint32) {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbroker_enable_auth_queue(
			_pointer, FfiConverterUint32INSTANCE.Lower(capacity), _uniffiStatus)
		return false
	})
}

// Pull-style alternative to [`set_event_listener`](Self::set_event_listener)
// for runtimes that can't receive callbacks from Rust threads (Dart,
// Haskell): keep up to `capacity` events for [`next_event`](Self::next_event).
// `0` turns the queue off; when it fills up the oldest event is dropped.
func (_self *MqttBroker) EnableEventQueue(capacity uint32) {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbroker_enable_event_queue(
			_pointer, FfiConverterUint32INSTANCE.Lower(capacity), _uniffiStatus)
		return false
	})
}

func (_self *MqttBroker) IsRunning() bool {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_mqtt_broker_fn_method_mqttbroker_is_running(
			_pointer, _uniffiStatus)
	}))
}

// The next CONNECT awaiting a decision, waiting up to `timeout_ms`
// milliseconds; `None` if none arrived. Requires
// [`enable_auth_queue`](Self::enable_auth_queue).
func (_self *MqttBroker) NextAuthRequest(timeoutMs uint32) *AuthRequest {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	res, _ := uniffiRustCallAsync[error](
		nil,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_mqtt_broker_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) *AuthRequest {
			return FfiConverterOptionalAuthRequestINSTANCE.Lift(ffi)
		},
		C.uniffi_mqtt_broker_fn_method_mqttbroker_next_auth_request(
			_pointer, FfiConverterUint32INSTANCE.Lower(timeoutMs)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_free_rust_buffer(handle)
		},
	)

	return res
}

// The next queued [`BrokerEvent`], waiting up to `timeout_ms`
// milliseconds; `None` if none arrived. Requires
// [`enable_event_queue`](Self::enable_event_queue).
func (_self *MqttBroker) NextEvent(timeoutMs uint32) *BrokerEvent {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	res, _ := uniffiRustCallAsync[error](
		nil,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_mqtt_broker_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) *BrokerEvent {
			return FfiConverterOptionalBrokerEventINSTANCE.Lift(ffi)
		},
		C.uniffi_mqtt_broker_fn_method_mqttbroker_next_event(
			_pointer, FfiConverterUint32INSTANCE.Lower(timeoutMs)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_free_rust_buffer(handle)
		},
	)

	return res
}

// Admit (`allow = true`) or refuse the CONNECT behind request `id`.
// Returns `false` if the request is unknown or already expired.
func (_self *MqttBroker) ResolveAuthRequest(id uint64, allow bool) bool {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_mqtt_broker_fn_method_mqttbroker_resolve_auth_request(
			_pointer, FfiConverterUint64INSTANCE.Lower(id), FfiConverterBoolINSTANCE.Lower(allow), _uniffiStatus)
	}))
}

// Register a pluggable authentication provider. Must be called before
// [`start`](Self::start) to affect connections made right away, but
// may be replaced at any time.
func (_self *MqttBroker) SetAuthProvider(provider MqttAuthProvider) {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbroker_set_auth_provider(
			_pointer, FfiConverterMqttAuthProviderINSTANCE.Lower(provider), _uniffiStatus)
		return false
	})
}

// Register the MQTT 5.0 enhanced-authentication provider (multi-round
// challenge/response during CONNECT). Without one, a CONNECT that names
// an `Authentication Method` is refused with "Bad authentication method".
func (_self *MqttBroker) SetEnhancedAuthProvider(provider MqttEnhancedAuthProvider) {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbroker_set_enhanced_auth_provider(
			_pointer, FfiConverterMqttEnhancedAuthProviderINSTANCE.Lower(provider), _uniffiStatus)
		return false
	})
}

// Register a listener for connect/disconnect/publish events.
func (_self *MqttBroker) SetEventListener(listener MqttBrokerEventListener) {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbroker_set_event_listener(
			_pointer, FfiConverterMqttBrokerEventListenerINSTANCE.Lower(listener), _uniffiStatus)
		return false
	})
}

// Bind the listening socket(s) and start accepting connections. Also
// binds a second, MQTT-over-WebSocket listener on `config.ws_port` and
// a TLS listener on `config.tls` when those are configured.
//
// All-or-nothing: every socket is bound (and the TLS configuration
// validated) *before* anything starts accepting, so a failure — say
// the WebSocket port is taken — leaves the broker cleanly stopped
// instead of half-running with some listeners already live.
func (_self *MqttBroker) Start() error {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[*MqttBrokerError](
		FfiConverterMqttBrokerErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_mqtt_broker_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_mqtt_broker_fn_method_mqttbroker_start(
			_pointer),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_free_void(handle)
		},
	)

	if err == nil {
		return nil
	}

	return err
}

// Stop the broker: close the listening sockets and ask every
// connected client's connection to shut down. Sessions that persist
// across disconnects (`clean_start = false`) are kept in memory, so a
// later [`start`](Self::start) resumes them. Calling `stop()` on a
// broker that isn't running is a no-op.
func (_self *MqttBroker) Stop() error {
	_pointer := _self.ffiObject.incrementPointer("*MqttBroker")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[*MqttBrokerError](
		FfiConverterMqttBrokerErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_mqtt_broker_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_mqtt_broker_fn_method_mqttbroker_stop(
			_pointer),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_mqtt_broker_rust_future_free_void(handle)
		},
	)

	if err == nil {
		return nil
	}

	return err
}
func (object *MqttBroker) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttBroker struct{}

var FfiConverterMqttBrokerINSTANCE = FfiConverterMqttBroker{}

func (c FfiConverterMqttBroker) Lift(handle C.uint64_t) *MqttBroker {
	result := &MqttBroker{
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_mqtt_broker_fn_clone_mqttbroker(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_mqtt_broker_fn_free_mqttbroker(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*MqttBroker).Destroy)
	return result
}

func (c FfiConverterMqttBroker) Read(reader io.Reader) *MqttBroker {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttBroker) Lower(value *MqttBroker) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*MqttBroker")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterMqttBroker) Write(writer io.Writer, value *MqttBroker) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttBroker(handle uint64) *MqttBroker {
	return FfiConverterMqttBrokerINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttBroker(value *MqttBroker) uint64 {
	return uint64(FfiConverterMqttBrokerINSTANCE.Lower(value))
}

type FfiDestroyerMqttBroker struct{}

func (_ FfiDestroyerMqttBroker) Destroy(value *MqttBroker) {
	value.Destroy()
}

// Broker-side view of a delivered/observed event, for monitoring and
// logging from foreign code.
type MqttBrokerEventListener interface {
	OnClientConnected(clientId string)
	OnClientDisconnected(clientId string, reason string)
	OnMessagePublished(clientId string, topic string, qos QoS)
}

// Broker-side view of a delivered/observed event, for monitoring and
// logging from foreign code.
type MqttBrokerEventListenerImpl struct {
	ffiObject FfiObject
}

func (_self *MqttBrokerEventListenerImpl) OnClientConnected(clientId string) {
	_pointer := _self.ffiObject.incrementPointer("MqttBrokerEventListener")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbrokereventlistener_on_client_connected(
			_pointer, FfiConverterStringINSTANCE.Lower(clientId), _uniffiStatus)
		return false
	})
}

func (_self *MqttBrokerEventListenerImpl) OnClientDisconnected(clientId string, reason string) {
	_pointer := _self.ffiObject.incrementPointer("MqttBrokerEventListener")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbrokereventlistener_on_client_disconnected(
			_pointer, FfiConverterStringINSTANCE.Lower(clientId), FfiConverterStringINSTANCE.Lower(reason), _uniffiStatus)
		return false
	})
}

func (_self *MqttBrokerEventListenerImpl) OnMessagePublished(clientId string, topic string, qos QoS) {
	_pointer := _self.ffiObject.incrementPointer("MqttBrokerEventListener")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_mqtt_broker_fn_method_mqttbrokereventlistener_on_message_published(
			_pointer, FfiConverterStringINSTANCE.Lower(clientId), FfiConverterStringINSTANCE.Lower(topic), FfiConverterQoSINSTANCE.Lower(qos), _uniffiStatus)
		return false
	})
}
func (object *MqttBrokerEventListenerImpl) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttBrokerEventListener struct {
	handleMap *concurrentHandleMap[MqttBrokerEventListener]
}

var FfiConverterMqttBrokerEventListenerINSTANCE = FfiConverterMqttBrokerEventListener{
	handleMap: newConcurrentHandleMap[MqttBrokerEventListener](),
}

func (c FfiConverterMqttBrokerEventListener) Lift(handle C.uint64_t) MqttBrokerEventListener {
	if uint64(handle)&1 == 0 {
		// Rust-generated handle (even), construct a new object wrapping the handle
		result := &MqttBrokerEventListenerImpl{
			newFfiObject(
				handle,
				func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
					return C.uniffi_mqtt_broker_fn_clone_mqttbrokereventlistener(handle, status)
				},
				func(handle C.uint64_t, status *C.RustCallStatus) {
					C.uniffi_mqtt_broker_fn_free_mqttbrokereventlistener(handle, status)
				},
			),
		}
		runtime.SetFinalizer(result, (*MqttBrokerEventListenerImpl).Destroy)
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

func (c FfiConverterMqttBrokerEventListener) Read(reader io.Reader) MqttBrokerEventListener {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttBrokerEventListener) Lower(value MqttBrokerEventListener) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	if val, ok := value.(*MqttBrokerEventListenerImpl); ok {
		// Rust-backed object, clone the handle
		handle := val.ffiObject.incrementPointer("MqttBrokerEventListener")
		defer val.ffiObject.decrementPointer()
		return handle
	} else {
		// Go-backed object, insert into handle map
		return C.uint64_t(c.handleMap.insert(value))
	}
}

func (c FfiConverterMqttBrokerEventListener) Write(writer io.Writer, value MqttBrokerEventListener) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttBrokerEventListener(handle uint64) MqttBrokerEventListener {
	return FfiConverterMqttBrokerEventListenerINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttBrokerEventListener(value MqttBrokerEventListener) uint64 {
	return uint64(FfiConverterMqttBrokerEventListenerINSTANCE.Lower(value))
}

type FfiDestroyerMqttBrokerEventListener struct{}

func (_ FfiDestroyerMqttBrokerEventListener) Destroy(value MqttBrokerEventListener) {
	if val, ok := value.(*MqttBrokerEventListenerImpl); ok {
		val.Destroy()
	}
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod0
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod0(uniffiHandle C.uint64_t, clientId C.RustBuffer, uniffiOutReturn *C.void, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttBrokerEventListenerINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	uniffiObj.OnClientConnected(
		FfiConverterStringINSTANCE.Lift(GoRustBuffer{
			inner: clientId,
		}),
	)

}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod1
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod1(uniffiHandle C.uint64_t, clientId C.RustBuffer, reason C.RustBuffer, uniffiOutReturn *C.void, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttBrokerEventListenerINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	uniffiObj.OnClientDisconnected(
		FfiConverterStringINSTANCE.Lift(GoRustBuffer{
			inner: clientId,
		}),
		FfiConverterStringINSTANCE.Lift(GoRustBuffer{
			inner: reason,
		}),
	)

}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod2
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod2(uniffiHandle C.uint64_t, clientId C.RustBuffer, topic C.RustBuffer, qos C.RustBuffer, uniffiOutReturn *C.void, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttBrokerEventListenerINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	uniffiObj.OnMessagePublished(
		FfiConverterStringINSTANCE.Lift(GoRustBuffer{
			inner: clientId,
		}),
		FfiConverterStringINSTANCE.Lift(GoRustBuffer{
			inner: topic,
		}),
		FfiConverterQoSINSTANCE.Lift(GoRustBuffer{
			inner: qos,
		}),
	)

}

var UniffiVTableCallbackInterfaceMqttBrokerEventListenerINSTANCE = C.UniffiVTableCallbackInterfaceMqttBrokerEventListener{
	uniffiFree:           (C.UniffiCallbackInterfaceFree)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerFree),
	uniffiClone:          (C.UniffiCallbackInterfaceClone)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerClone),
	onClientConnected:    (C.UniffiCallbackInterfaceMqttBrokerEventListenerMethod0)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod0),
	onClientDisconnected: (C.UniffiCallbackInterfaceMqttBrokerEventListenerMethod1)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod1),
	onMessagePublished:   (C.UniffiCallbackInterfaceMqttBrokerEventListenerMethod2)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerMethod2),
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerFree
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerFree(handle C.uint64_t) {
	FfiConverterMqttBrokerEventListenerINSTANCE.handleMap.remove(uint64(handle))
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerClone
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttBrokerEventListenerClone(handle C.uint64_t) C.uint64_t {
	val, ok := FfiConverterMqttBrokerEventListenerINSTANCE.handleMap.tryGet(uint64(handle))
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return C.uint64_t(FfiConverterMqttBrokerEventListenerINSTANCE.handleMap.insert(val))
}

func (c FfiConverterMqttBrokerEventListener) register() {
	C.uniffi_mqtt_broker_fn_init_callback_vtable_mqttbrokereventlistener(&UniffiVTableCallbackInterfaceMqttBrokerEventListenerINSTANCE)
}

// Pluggable MQTT 5.0 *enhanced* authentication (SCRAM, Kerberos, OAuth, …):
// a multi-round challenge/response evaluated during CONNECT, before the
// session is attached. A CONNECT naming an `Authentication Method` is refused
// unless a provider is registered.
type MqttEnhancedAuthProvider interface {
	// One round. `round` is `0` for the CONNECT's own `Authentication Data`
	// (`data`, possibly `None`) and `1, 2, …` for each client response.
	Step(clientId string, method string, data *[]byte, round uint32) EnhancedAuthStep
}

// Pluggable MQTT 5.0 *enhanced* authentication (SCRAM, Kerberos, OAuth, …):
// a multi-round challenge/response evaluated during CONNECT, before the
// session is attached. A CONNECT naming an `Authentication Method` is refused
// unless a provider is registered.
type MqttEnhancedAuthProviderImpl struct {
	ffiObject FfiObject
}

// One round. `round` is `0` for the CONNECT's own `Authentication Data`
// (`data`, possibly `None`) and `1, 2, …` for each client response.
func (_self *MqttEnhancedAuthProviderImpl) Step(clientId string, method string, data *[]byte, round uint32) EnhancedAuthStep {
	_pointer := _self.ffiObject.incrementPointer("MqttEnhancedAuthProvider")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterEnhancedAuthStepINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_mqtt_broker_fn_method_mqttenhancedauthprovider_step(
				_pointer, FfiConverterStringINSTANCE.Lower(clientId), FfiConverterStringINSTANCE.Lower(method), FfiConverterOptionalBytesINSTANCE.Lower(data), FfiConverterUint32INSTANCE.Lower(round), _uniffiStatus),
		}
	}))
}
func (object *MqttEnhancedAuthProviderImpl) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMqttEnhancedAuthProvider struct {
	handleMap *concurrentHandleMap[MqttEnhancedAuthProvider]
}

var FfiConverterMqttEnhancedAuthProviderINSTANCE = FfiConverterMqttEnhancedAuthProvider{
	handleMap: newConcurrentHandleMap[MqttEnhancedAuthProvider](),
}

func (c FfiConverterMqttEnhancedAuthProvider) Lift(handle C.uint64_t) MqttEnhancedAuthProvider {
	if uint64(handle)&1 == 0 {
		// Rust-generated handle (even), construct a new object wrapping the handle
		result := &MqttEnhancedAuthProviderImpl{
			newFfiObject(
				handle,
				func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
					return C.uniffi_mqtt_broker_fn_clone_mqttenhancedauthprovider(handle, status)
				},
				func(handle C.uint64_t, status *C.RustCallStatus) {
					C.uniffi_mqtt_broker_fn_free_mqttenhancedauthprovider(handle, status)
				},
			),
		}
		runtime.SetFinalizer(result, (*MqttEnhancedAuthProviderImpl).Destroy)
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

func (c FfiConverterMqttEnhancedAuthProvider) Read(reader io.Reader) MqttEnhancedAuthProvider {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMqttEnhancedAuthProvider) Lower(value MqttEnhancedAuthProvider) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	if val, ok := value.(*MqttEnhancedAuthProviderImpl); ok {
		// Rust-backed object, clone the handle
		handle := val.ffiObject.incrementPointer("MqttEnhancedAuthProvider")
		defer val.ffiObject.decrementPointer()
		return handle
	} else {
		// Go-backed object, insert into handle map
		return C.uint64_t(c.handleMap.insert(value))
	}
}

func (c FfiConverterMqttEnhancedAuthProvider) Write(writer io.Writer, value MqttEnhancedAuthProvider) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMqttEnhancedAuthProvider(handle uint64) MqttEnhancedAuthProvider {
	return FfiConverterMqttEnhancedAuthProviderINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMqttEnhancedAuthProvider(value MqttEnhancedAuthProvider) uint64 {
	return uint64(FfiConverterMqttEnhancedAuthProviderINSTANCE.Lower(value))
}

type FfiDestroyerMqttEnhancedAuthProvider struct{}

func (_ FfiDestroyerMqttEnhancedAuthProvider) Destroy(value MqttEnhancedAuthProvider) {
	if val, ok := value.(*MqttEnhancedAuthProviderImpl); ok {
		val.Destroy()
	}
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderMethod0
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderMethod0(uniffiHandle C.uint64_t, clientId C.RustBuffer, method C.RustBuffer, data C.RustBuffer, round C.uint32_t, uniffiOutReturn *C.RustBuffer, callStatus *C.RustCallStatus) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterMqttEnhancedAuthProviderINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}

	res :=
		uniffiObj.Step(
			FfiConverterStringINSTANCE.Lift(GoRustBuffer{
				inner: clientId,
			}),
			FfiConverterStringINSTANCE.Lift(GoRustBuffer{
				inner: method,
			}),
			FfiConverterOptionalBytesINSTANCE.Lift(GoRustBuffer{
				inner: data,
			}),
			FfiConverterUint32INSTANCE.Lift(round),
		)

	*uniffiOutReturn = FfiConverterEnhancedAuthStepINSTANCE.Lower(res)
}

var UniffiVTableCallbackInterfaceMqttEnhancedAuthProviderINSTANCE = C.UniffiVTableCallbackInterfaceMqttEnhancedAuthProvider{
	uniffiFree:  (C.UniffiCallbackInterfaceFree)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderFree),
	uniffiClone: (C.UniffiCallbackInterfaceClone)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderClone),
	step:        (C.UniffiCallbackInterfaceMqttEnhancedAuthProviderMethod0)(C.mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderMethod0),
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderFree
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderFree(handle C.uint64_t) {
	FfiConverterMqttEnhancedAuthProviderINSTANCE.handleMap.remove(uint64(handle))
}

//export mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderClone
func mqtt_broker_config_cgo_dispatchCallbackInterfaceMqttEnhancedAuthProviderClone(handle C.uint64_t) C.uint64_t {
	val, ok := FfiConverterMqttEnhancedAuthProviderINSTANCE.handleMap.tryGet(uint64(handle))
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return C.uint64_t(FfiConverterMqttEnhancedAuthProviderINSTANCE.handleMap.insert(val))
}

func (c FfiConverterMqttEnhancedAuthProvider) register() {
	C.uniffi_mqtt_broker_fn_init_callback_vtable_mqttenhancedauthprovider(&UniffiVTableCallbackInterfaceMqttEnhancedAuthProviderINSTANCE)
}

// One CONNECT awaiting an allow/deny decision.
type AuthRequest struct {
	// Pass this to `resolve_auth_request`.
	Id       uint64
	ClientId string
	Username *string
	Password *[]byte
}

func (r *AuthRequest) Destroy() {
	FfiDestroyerUint64{}.Destroy(r.Id)
	FfiDestroyerString{}.Destroy(r.ClientId)
	FfiDestroyerOptionalString{}.Destroy(r.Username)
	FfiDestroyerOptionalBytes{}.Destroy(r.Password)
}

type FfiConverterAuthRequest struct{}

var FfiConverterAuthRequestINSTANCE = FfiConverterAuthRequest{}

func (c FfiConverterAuthRequest) Lift(rb RustBufferI) AuthRequest {
	return LiftFromRustBuffer[AuthRequest](c, rb)
}

func (c FfiConverterAuthRequest) Read(reader io.Reader) AuthRequest {
	return AuthRequest{
		FfiConverterUint64INSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalBytesINSTANCE.Read(reader),
	}
}

func (c FfiConverterAuthRequest) Lower(value AuthRequest) C.RustBuffer {
	return LowerIntoRustBuffer[AuthRequest](c, value)
}

func (c FfiConverterAuthRequest) LowerExternal(value AuthRequest) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[AuthRequest](c, value))
}

func (c FfiConverterAuthRequest) Write(writer io.Writer, value AuthRequest) {
	FfiConverterUint64INSTANCE.Write(writer, value.Id)
	FfiConverterStringINSTANCE.Write(writer, value.ClientId)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Username)
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.Password)
}

type FfiDestroyerAuthRequest struct{}

func (_ FfiDestroyerAuthRequest) Destroy(value AuthRequest) {
	value.Destroy()
}

// TLS configuration for [`MqttBrokerConfig::tls`]. All certificate/key
// material is PEM-encoded bytes (not file paths), matching
// `mqtt_client::TlsOptions` so this works identically across every
// language binding without assuming a filesystem layout.
type BrokerTlsConfig struct {
	// Port to accept TLS connections on.
	Port uint16
	// PEM-encoded server certificate chain (leaf certificate first, then
	// any intermediates).
	CertPem []byte
	// PEM-encoded server private key.
	KeyPem []byte
	// PEM-encoded CA certificate(s) to verify *client* certificates
	// against, for mutual TLS (mTLS). If unset (the default), client
	// certificates aren't required or checked — just a normal one-way
	// TLS listener.
	ClientCaPem *[]byte
}

func (r *BrokerTlsConfig) Destroy() {
	FfiDestroyerUint16{}.Destroy(r.Port)
	FfiDestroyerBytes{}.Destroy(r.CertPem)
	FfiDestroyerBytes{}.Destroy(r.KeyPem)
	FfiDestroyerOptionalBytes{}.Destroy(r.ClientCaPem)
}

type FfiConverterBrokerTlsConfig struct{}

var FfiConverterBrokerTlsConfigINSTANCE = FfiConverterBrokerTlsConfig{}

func (c FfiConverterBrokerTlsConfig) Lift(rb RustBufferI) BrokerTlsConfig {
	return LiftFromRustBuffer[BrokerTlsConfig](c, rb)
}

func (c FfiConverterBrokerTlsConfig) Read(reader io.Reader) BrokerTlsConfig {
	return BrokerTlsConfig{
		FfiConverterUint16INSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
		FfiConverterOptionalBytesINSTANCE.Read(reader),
	}
}

func (c FfiConverterBrokerTlsConfig) Lower(value BrokerTlsConfig) C.RustBuffer {
	return LowerIntoRustBuffer[BrokerTlsConfig](c, value)
}

func (c FfiConverterBrokerTlsConfig) LowerExternal(value BrokerTlsConfig) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[BrokerTlsConfig](c, value))
}

func (c FfiConverterBrokerTlsConfig) Write(writer io.Writer, value BrokerTlsConfig) {
	FfiConverterUint16INSTANCE.Write(writer, value.Port)
	FfiConverterBytesINSTANCE.Write(writer, value.CertPem)
	FfiConverterBytesINSTANCE.Write(writer, value.KeyPem)
	FfiConverterOptionalBytesINSTANCE.Write(writer, value.ClientCaPem)
}

type FfiDestroyerBrokerTlsConfig struct{}

func (_ FfiDestroyerBrokerTlsConfig) Destroy(value BrokerTlsConfig) {
	value.Destroy()
}

// What the broker should do after one authentication round — a record
// (outcome + payload) rather than an enum-with-data, because several
// binding generators mishandle callbacks that return data-carrying enums.
type EnhancedAuthStep struct {
	Outcome EnhancedAuthOutcome
	// Challenge (for `Continue`) or final data (for `Success`); empty = none.
	Data []byte
}

func (r *EnhancedAuthStep) Destroy() {
	FfiDestroyerEnhancedAuthOutcome{}.Destroy(r.Outcome)
	FfiDestroyerBytes{}.Destroy(r.Data)
}

type FfiConverterEnhancedAuthStep struct{}

var FfiConverterEnhancedAuthStepINSTANCE = FfiConverterEnhancedAuthStep{}

func (c FfiConverterEnhancedAuthStep) Lift(rb RustBufferI) EnhancedAuthStep {
	return LiftFromRustBuffer[EnhancedAuthStep](c, rb)
}

func (c FfiConverterEnhancedAuthStep) Read(reader io.Reader) EnhancedAuthStep {
	return EnhancedAuthStep{
		FfiConverterEnhancedAuthOutcomeINSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
	}
}

func (c FfiConverterEnhancedAuthStep) Lower(value EnhancedAuthStep) C.RustBuffer {
	return LowerIntoRustBuffer[EnhancedAuthStep](c, value)
}

func (c FfiConverterEnhancedAuthStep) LowerExternal(value EnhancedAuthStep) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[EnhancedAuthStep](c, value))
}

func (c FfiConverterEnhancedAuthStep) Write(writer io.Writer, value EnhancedAuthStep) {
	FfiConverterEnhancedAuthOutcomeINSTANCE.Write(writer, value.Outcome)
	FfiConverterBytesINSTANCE.Write(writer, value.Data)
}

type FfiDestroyerEnhancedAuthStep struct{}

func (_ FfiDestroyerEnhancedAuthStep) Destroy(value EnhancedAuthStep) {
	value.Destroy()
}

// Configuration for an [`crate::broker::MqttBroker`].
type MqttBrokerConfig struct {
	// Address to bind the listening socket to, e.g. `"0.0.0.0"` or `"::"`.
	BindAddress string
	Port        uint16
	// Port to accept MQTT-over-WebSocket connections on (MQTT-5.0 §6 /
	// MQTT-3.1.1 Appendix B), bound alongside the raw-TCP `port` above.
	// `None` disables the WebSocket listener entirely — the default,
	// since most deployments only need raw TCP. `Some(0)` binds an
	// OS-assigned ephemeral WebSocket port (mirroring `port`'s own `0`
	// convention), which is why this is `Option<u16>` rather than reusing
	// `0` itself as the "disabled" sentinel.
	WsPort *uint16
	// With no [`MqttAuthProvider`] registered, whether *everyone* may connect
	// (`true`) or *no one* may (`false`) — there is nothing to verify a
	// username against, so supplying one grants nothing. Ignored if an auth
	// provider is set — the provider always makes the final decision.
	AllowAnonymous bool
	// Maximum simultaneously connected clients. `0` means unlimited.
	MaxClients uint32
	// Highest QoS the broker will grant on SUBSCRIBE (messages published
	// at a higher QoS are still accepted, but delivered at this ceiling).
	MaxQos QoS
	// Maximum number of retained messages kept in memory. `0` means
	// unlimited.
	MaxRetainedMessages uint32
	// Maximum number of messages queued per offline session (for clients
	// that connected with `clean_start = false`). `0` means unlimited.
	MaxQueuedPerClient uint32
	// How often the broker resends (with DUP=1) an outgoing QoS 1/2
	// packet that hasn't been acked yet. `0` selects the built-in default
	// (5 seconds).
	RedeliveryIntervalSecs uint32
	// If set, also accept TLS (`mqtts`) connections on
	// [`BrokerTlsConfig::port`], alongside the plain-TCP `port` (and
	// optional WebSocket `ws_port`) above. `None` (the default) means no
	// TLS listener at all, unchanged from before TLS support existed.
	Tls *BrokerTlsConfig
	// Largest packet (fixed header + body, in bytes) accepted from any
	// client. A client announcing a bigger one is disconnected as soon as
	// the packet *header* is read, before the body is buffered. `0`
	// selects the built-in default
	// ([`DEFAULT_MAX_PACKET_SIZE`], 1 MiB).
	MaxPacketSize uint32
	// Maximum encoded packets queued for one client's socket. A client
	// that reads slower than messages arrive would otherwise make the
	// broker buffer without bound; once this many are waiting, further
	// deliveries to *that* client are dropped (QoS 1/2 ones are retried
	// by the redelivery sweep). `0` selects the built-in default
	// ([`DEFAULT_MAX_OUTBOUND_QUEUE`], 4096).
	MaxOutboundQueue uint32
	// Seconds a persistent (`clean_start = false`) session may stay offline
	// before the broker discards it, with its subscriptions and queued
	// messages. `0` (the default) keeps sessions forever, as before.
	SessionExpirySecs uint32
}

func (r *MqttBrokerConfig) Destroy() {
	FfiDestroyerString{}.Destroy(r.BindAddress)
	FfiDestroyerUint16{}.Destroy(r.Port)
	FfiDestroyerOptionalUint16{}.Destroy(r.WsPort)
	FfiDestroyerBool{}.Destroy(r.AllowAnonymous)
	FfiDestroyerUint32{}.Destroy(r.MaxClients)
	FfiDestroyerQoS{}.Destroy(r.MaxQos)
	FfiDestroyerUint32{}.Destroy(r.MaxRetainedMessages)
	FfiDestroyerUint32{}.Destroy(r.MaxQueuedPerClient)
	FfiDestroyerUint32{}.Destroy(r.RedeliveryIntervalSecs)
	FfiDestroyerOptionalBrokerTlsConfig{}.Destroy(r.Tls)
	FfiDestroyerUint32{}.Destroy(r.MaxPacketSize)
	FfiDestroyerUint32{}.Destroy(r.MaxOutboundQueue)
	FfiDestroyerUint32{}.Destroy(r.SessionExpirySecs)
}

type FfiConverterMqttBrokerConfig struct{}

var FfiConverterMqttBrokerConfigINSTANCE = FfiConverterMqttBrokerConfig{}

func (c FfiConverterMqttBrokerConfig) Lift(rb RustBufferI) MqttBrokerConfig {
	return LiftFromRustBuffer[MqttBrokerConfig](c, rb)
}

func (c FfiConverterMqttBrokerConfig) Read(reader io.Reader) MqttBrokerConfig {
	return MqttBrokerConfig{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterUint16INSTANCE.Read(reader),
		FfiConverterOptionalUint16INSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterQoSINSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterOptionalBrokerTlsConfigINSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
		FfiConverterUint32INSTANCE.Read(reader),
	}
}

func (c FfiConverterMqttBrokerConfig) Lower(value MqttBrokerConfig) C.RustBuffer {
	return LowerIntoRustBuffer[MqttBrokerConfig](c, value)
}

func (c FfiConverterMqttBrokerConfig) LowerExternal(value MqttBrokerConfig) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[MqttBrokerConfig](c, value))
}

func (c FfiConverterMqttBrokerConfig) Write(writer io.Writer, value MqttBrokerConfig) {
	FfiConverterStringINSTANCE.Write(writer, value.BindAddress)
	FfiConverterUint16INSTANCE.Write(writer, value.Port)
	FfiConverterOptionalUint16INSTANCE.Write(writer, value.WsPort)
	FfiConverterBoolINSTANCE.Write(writer, value.AllowAnonymous)
	FfiConverterUint32INSTANCE.Write(writer, value.MaxClients)
	FfiConverterQoSINSTANCE.Write(writer, value.MaxQos)
	FfiConverterUint32INSTANCE.Write(writer, value.MaxRetainedMessages)
	FfiConverterUint32INSTANCE.Write(writer, value.MaxQueuedPerClient)
	FfiConverterUint32INSTANCE.Write(writer, value.RedeliveryIntervalSecs)
	FfiConverterOptionalBrokerTlsConfigINSTANCE.Write(writer, value.Tls)
	FfiConverterUint32INSTANCE.Write(writer, value.MaxPacketSize)
	FfiConverterUint32INSTANCE.Write(writer, value.MaxOutboundQueue)
	FfiConverterUint32INSTANCE.Write(writer, value.SessionExpirySecs)
}

type FfiDestroyerMqttBrokerConfig struct{}

func (_ FfiDestroyerMqttBrokerConfig) Destroy(value MqttBrokerConfig) {
	value.Destroy()
}

// A broker event, as returned by the pull-style `MqttBroker::next_event`
// (the polling twin of [`MqttBrokerEventListener`](crate::config::MqttBrokerEventListener)).
type BrokerEvent interface {
	Destroy()
}
type BrokerEventClientConnected struct {
	ClientId string
}

func (e BrokerEventClientConnected) Destroy() {
	FfiDestroyerString{}.Destroy(e.ClientId)
}

type BrokerEventClientDisconnected struct {
	ClientId string
	Reason   string
}

func (e BrokerEventClientDisconnected) Destroy() {
	FfiDestroyerString{}.Destroy(e.ClientId)
	FfiDestroyerString{}.Destroy(e.Reason)
}

type BrokerEventMessagePublished struct {
	ClientId string
	Topic    string
	Qos      QoS
}

func (e BrokerEventMessagePublished) Destroy() {
	FfiDestroyerString{}.Destroy(e.ClientId)
	FfiDestroyerString{}.Destroy(e.Topic)
	FfiDestroyerQoS{}.Destroy(e.Qos)
}

type FfiConverterBrokerEvent struct{}

var FfiConverterBrokerEventINSTANCE = FfiConverterBrokerEvent{}

func (c FfiConverterBrokerEvent) Lift(rb RustBufferI) BrokerEvent {
	return LiftFromRustBuffer[BrokerEvent](c, rb)
}

func (c FfiConverterBrokerEvent) Lower(value BrokerEvent) C.RustBuffer {
	return LowerIntoRustBuffer[BrokerEvent](c, value)
}

func (c FfiConverterBrokerEvent) LowerExternal(value BrokerEvent) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[BrokerEvent](c, value))
}
func (FfiConverterBrokerEvent) Read(reader io.Reader) BrokerEvent {
	id := readInt32(reader)
	switch id {
	case 1:
		return BrokerEventClientConnected{
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 2:
		return BrokerEventClientDisconnected{
			FfiConverterStringINSTANCE.Read(reader),
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 3:
		return BrokerEventMessagePublished{
			FfiConverterStringINSTANCE.Read(reader),
			FfiConverterStringINSTANCE.Read(reader),
			FfiConverterQoSINSTANCE.Read(reader),
		}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterBrokerEvent.Read()", id))
	}
}

func (FfiConverterBrokerEvent) Write(writer io.Writer, value BrokerEvent) {
	switch variant_value := value.(type) {
	case BrokerEventClientConnected:
		writeInt32(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, variant_value.ClientId)
	case BrokerEventClientDisconnected:
		writeInt32(writer, 2)
		FfiConverterStringINSTANCE.Write(writer, variant_value.ClientId)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Reason)
	case BrokerEventMessagePublished:
		writeInt32(writer, 3)
		FfiConverterStringINSTANCE.Write(writer, variant_value.ClientId)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Topic)
		FfiConverterQoSINSTANCE.Write(writer, variant_value.Qos)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterBrokerEvent.Write", value))
	}
}

type FfiDestroyerBrokerEvent struct{}

func (_ FfiDestroyerBrokerEvent) Destroy(value BrokerEvent) {
	value.Destroy()
}

// The verdict of one round of an MQTT 5.0 enhanced authentication exchange.
type EnhancedAuthOutcome uint

const (
	// Send [`EnhancedAuthStep::data`] to the client as a challenge and wait
	// for its response.
	EnhancedAuthOutcomeContinue EnhancedAuthOutcome = 1
	// Authentication succeeded; [`EnhancedAuthStep::data`] (if non-empty) is
	// returned to the client in the CONNACK (e.g. a SCRAM server-final message).
	EnhancedAuthOutcomeSuccess EnhancedAuthOutcome = 2
	// Refuse the connection.
	EnhancedAuthOutcomeFailure EnhancedAuthOutcome = 3
)

type FfiConverterEnhancedAuthOutcome struct{}

var FfiConverterEnhancedAuthOutcomeINSTANCE = FfiConverterEnhancedAuthOutcome{}

func (c FfiConverterEnhancedAuthOutcome) Lift(rb RustBufferI) EnhancedAuthOutcome {
	return LiftFromRustBuffer[EnhancedAuthOutcome](c, rb)
}

func (c FfiConverterEnhancedAuthOutcome) Lower(value EnhancedAuthOutcome) C.RustBuffer {
	return LowerIntoRustBuffer[EnhancedAuthOutcome](c, value)
}

func (c FfiConverterEnhancedAuthOutcome) LowerExternal(value EnhancedAuthOutcome) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[EnhancedAuthOutcome](c, value))
}
func (FfiConverterEnhancedAuthOutcome) Read(reader io.Reader) EnhancedAuthOutcome {
	id := readInt32(reader)
	return EnhancedAuthOutcome(id)
}

func (FfiConverterEnhancedAuthOutcome) Write(writer io.Writer, value EnhancedAuthOutcome) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerEnhancedAuthOutcome struct{}

func (_ FfiDestroyerEnhancedAuthOutcome) Destroy(value EnhancedAuthOutcome) {
}

// Errors produced while starting or stopping an [`crate::MqttBroker`].
type MqttBrokerError struct {
	err error
}

// Convenience method to turn *MqttBrokerError into error
// Avoiding treating nil pointer as non nil error interface
func (err *MqttBrokerError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err MqttBrokerError) Error() string {
	return fmt.Sprintf("MqttBrokerError: %s", err.err.Error())
}

func (err MqttBrokerError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrMqttBrokerErrorAlreadyRunning = fmt.Errorf("MqttBrokerErrorAlreadyRunning")
var ErrMqttBrokerErrorIo = fmt.Errorf("MqttBrokerErrorIo")
var ErrMqttBrokerErrorTls = fmt.Errorf("MqttBrokerErrorTls")

// Variant structs
// `start()` was called while the broker was already running.
type MqttBrokerErrorAlreadyRunning struct {
	message string
}

// `start()` was called while the broker was already running.
func NewMqttBrokerErrorAlreadyRunning() *MqttBrokerError {
	return &MqttBrokerError{err: &MqttBrokerErrorAlreadyRunning{}}
}

func (e MqttBrokerErrorAlreadyRunning) destroy() {
}

func (err MqttBrokerErrorAlreadyRunning) Error() string {
	return fmt.Sprintf("AlreadyRunning: %s", err.message)
}

func (self MqttBrokerErrorAlreadyRunning) Is(target error) bool {
	return target == ErrMqttBrokerErrorAlreadyRunning
}

// A listening socket could not be bound or queried.
type MqttBrokerErrorIo struct {
	message string
}

// A listening socket could not be bound or queried.
func NewMqttBrokerErrorIo() *MqttBrokerError {
	return &MqttBrokerError{err: &MqttBrokerErrorIo{}}
}

func (e MqttBrokerErrorIo) destroy() {
}

func (err MqttBrokerErrorIo) Error() string {
	return fmt.Sprintf("Io: %s", err.message)
}

func (self MqttBrokerErrorIo) Is(target error) bool {
	return target == ErrMqttBrokerErrorIo
}

// TLS certificate, key, or verifier configuration was invalid.
type MqttBrokerErrorTls struct {
	message string
}

// TLS certificate, key, or verifier configuration was invalid.
func NewMqttBrokerErrorTls() *MqttBrokerError {
	return &MqttBrokerError{err: &MqttBrokerErrorTls{}}
}

func (e MqttBrokerErrorTls) destroy() {
}

func (err MqttBrokerErrorTls) Error() string {
	return fmt.Sprintf("Tls: %s", err.message)
}

func (self MqttBrokerErrorTls) Is(target error) bool {
	return target == ErrMqttBrokerErrorTls
}

type FfiConverterMqttBrokerError struct{}

var FfiConverterMqttBrokerErrorINSTANCE = FfiConverterMqttBrokerError{}

func (c FfiConverterMqttBrokerError) Lift(eb RustBufferI) *MqttBrokerError {
	return LiftFromRustBuffer[*MqttBrokerError](c, eb)
}

func (c FfiConverterMqttBrokerError) Lower(value *MqttBrokerError) C.RustBuffer {
	return LowerIntoRustBuffer[*MqttBrokerError](c, value)
}

func (c FfiConverterMqttBrokerError) LowerExternal(value *MqttBrokerError) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*MqttBrokerError](c, value))
}

func (c FfiConverterMqttBrokerError) Read(reader io.Reader) *MqttBrokerError {
	errorID := readUint32(reader)

	message := FfiConverterStringINSTANCE.Read(reader)
	switch errorID {
	case 1:
		return &MqttBrokerError{&MqttBrokerErrorAlreadyRunning{message}}
	case 2:
		return &MqttBrokerError{&MqttBrokerErrorIo{message}}
	case 3:
		return &MqttBrokerError{&MqttBrokerErrorTls{message}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterMqttBrokerError.Read()", errorID))
	}

}

func (c FfiConverterMqttBrokerError) Write(writer io.Writer, value *MqttBrokerError) {
	switch variantValue := value.err.(type) {
	case *MqttBrokerErrorAlreadyRunning:
		writeInt32(writer, 1)
	case *MqttBrokerErrorIo:
		writeInt32(writer, 2)
	case *MqttBrokerErrorTls:
		writeInt32(writer, 3)
	default:
		_ = variantValue
		panic(fmt.Sprintf("invalid error value `%v` in FfiConverterMqttBrokerError.Write", value))
	}
}

type FfiDestroyerMqttBrokerError struct{}

func (_ FfiDestroyerMqttBrokerError) Destroy(value *MqttBrokerError) {
	switch variantValue := value.err.(type) {
	case MqttBrokerErrorAlreadyRunning:
		variantValue.destroy()
	case MqttBrokerErrorIo:
		variantValue.destroy()
	case MqttBrokerErrorTls:
		variantValue.destroy()
	default:
		_ = variantValue
		panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerMqttBrokerError.Destroy", value))
	}
}

// MQTT Quality of Service level, as used in the broker's own API
// ([`MqttBrokerConfig::max_qos`], [`MqttBrokerEventListener`], [`crate::BrokerEvent`]).
//
// This is deliberately the **broker's own** enum rather than a re-export of
// `mqtt_client::QoS`. A type defined in another crate is an *external type*
// to UniFFI, and several binding generators (Node.js) can't handle those;
// owning it makes `mqtt_broker`'s bindings fully self-contained in every
// language — one package, no dependency on the client component's generated
// code. The cost is a second `QoS` next to the client's (identical, with
// lossless `From` conversions in Rust).
type QoS uint

const (
	// QoS 0 — fire and forget.
	QoSAtMostOnce QoS = 1
	// QoS 1 — acknowledged delivery, may be duplicated.
	QoSAtLeastOnce QoS = 2
	// QoS 2 — exactly-once delivery.
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

type FfiConverterOptionalUint16 struct{}

var FfiConverterOptionalUint16INSTANCE = FfiConverterOptionalUint16{}

func (c FfiConverterOptionalUint16) Lift(rb RustBufferI) *uint16 {
	return LiftFromRustBuffer[*uint16](c, rb)
}

func (_ FfiConverterOptionalUint16) Read(reader io.Reader) *uint16 {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterUint16INSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalUint16) Lower(value *uint16) C.RustBuffer {
	return LowerIntoRustBuffer[*uint16](c, value)
}

func (c FfiConverterOptionalUint16) LowerExternal(value *uint16) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*uint16](c, value))
}

func (_ FfiConverterOptionalUint16) Write(writer io.Writer, value *uint16) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterUint16INSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalUint16 struct{}

func (_ FfiDestroyerOptionalUint16) Destroy(value *uint16) {
	if value != nil {
		FfiDestroyerUint16{}.Destroy(*value)
	}
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

type FfiConverterOptionalAuthRequest struct{}

var FfiConverterOptionalAuthRequestINSTANCE = FfiConverterOptionalAuthRequest{}

func (c FfiConverterOptionalAuthRequest) Lift(rb RustBufferI) *AuthRequest {
	return LiftFromRustBuffer[*AuthRequest](c, rb)
}

func (_ FfiConverterOptionalAuthRequest) Read(reader io.Reader) *AuthRequest {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterAuthRequestINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalAuthRequest) Lower(value *AuthRequest) C.RustBuffer {
	return LowerIntoRustBuffer[*AuthRequest](c, value)
}

func (c FfiConverterOptionalAuthRequest) LowerExternal(value *AuthRequest) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*AuthRequest](c, value))
}

func (_ FfiConverterOptionalAuthRequest) Write(writer io.Writer, value *AuthRequest) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterAuthRequestINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalAuthRequest struct{}

func (_ FfiDestroyerOptionalAuthRequest) Destroy(value *AuthRequest) {
	if value != nil {
		FfiDestroyerAuthRequest{}.Destroy(*value)
	}
}

type FfiConverterOptionalBrokerTlsConfig struct{}

var FfiConverterOptionalBrokerTlsConfigINSTANCE = FfiConverterOptionalBrokerTlsConfig{}

func (c FfiConverterOptionalBrokerTlsConfig) Lift(rb RustBufferI) *BrokerTlsConfig {
	return LiftFromRustBuffer[*BrokerTlsConfig](c, rb)
}

func (_ FfiConverterOptionalBrokerTlsConfig) Read(reader io.Reader) *BrokerTlsConfig {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterBrokerTlsConfigINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalBrokerTlsConfig) Lower(value *BrokerTlsConfig) C.RustBuffer {
	return LowerIntoRustBuffer[*BrokerTlsConfig](c, value)
}

func (c FfiConverterOptionalBrokerTlsConfig) LowerExternal(value *BrokerTlsConfig) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*BrokerTlsConfig](c, value))
}

func (_ FfiConverterOptionalBrokerTlsConfig) Write(writer io.Writer, value *BrokerTlsConfig) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterBrokerTlsConfigINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalBrokerTlsConfig struct{}

func (_ FfiDestroyerOptionalBrokerTlsConfig) Destroy(value *BrokerTlsConfig) {
	if value != nil {
		FfiDestroyerBrokerTlsConfig{}.Destroy(*value)
	}
}

type FfiConverterOptionalBrokerEvent struct{}

var FfiConverterOptionalBrokerEventINSTANCE = FfiConverterOptionalBrokerEvent{}

func (c FfiConverterOptionalBrokerEvent) Lift(rb RustBufferI) *BrokerEvent {
	return LiftFromRustBuffer[*BrokerEvent](c, rb)
}

func (_ FfiConverterOptionalBrokerEvent) Read(reader io.Reader) *BrokerEvent {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterBrokerEventINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalBrokerEvent) Lower(value *BrokerEvent) C.RustBuffer {
	return LowerIntoRustBuffer[*BrokerEvent](c, value)
}

func (c FfiConverterOptionalBrokerEvent) LowerExternal(value *BrokerEvent) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*BrokerEvent](c, value))
}

func (_ FfiConverterOptionalBrokerEvent) Write(writer io.Writer, value *BrokerEvent) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterBrokerEventINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalBrokerEvent struct{}

func (_ FfiDestroyerOptionalBrokerEvent) Destroy(value *BrokerEvent) {
	if value != nil {
		FfiDestroyerBrokerEvent{}.Destroy(*value)
	}
}

const (
	uniffiRustFuturePollReady      int8 = 0
	uniffiRustFuturePollMaybeReady int8 = 1
)

type rustFuturePollFunc func(C.uint64_t, C.UniffiRustFutureContinuationCallback, C.uint64_t)
type rustFutureCompleteFunc[T any] func(C.uint64_t, *C.RustCallStatus) T
type rustFutureFreeFunc func(C.uint64_t)

//export mqtt_broker_uniffiFutureContinuationCallback
func mqtt_broker_uniffiFutureContinuationCallback(data C.uint64_t, pollResult C.int8_t) {
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
			(C.UniffiRustFutureContinuationCallback)(C.mqtt_broker_uniffiFutureContinuationCallback),
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

//export mqtt_broker_uniffiFreeGorutine
func mqtt_broker_uniffiFreeGorutine(data C.uint64_t) {
	handle := cgo.Handle(uintptr(data))
	defer handle.Delete()

	guard := handle.Value().(chan struct{})
	guard <- struct{}{}
}
