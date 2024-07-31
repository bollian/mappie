from adafruit_motorkit import MotorKit
from time import sleep

BT_SERVICE_UUID = "94f39d29-7d6d-437d-973b-fba39e49d4ee"

def main():
    # start the bluetooth connection
    bt_sock = bluetooth.BluetoothSocket(bluetooth.RFCOMM)
    bt_sock.bind(("", bluetooth.PORT_ANY))
    bt_sock.listen(1) # at most 1 pending connections
    bluetooth.advertise_service(bt_sock, "Mappie Robot",
        service_id=BT_SERVICE_UUID,
        service_classes=[BT_SERVICE_UUID, bluetooth.SERIAL_PORT_CLASS],
        profiles=[bluetooth.SERIAL_PORT_PROFILE],
        # protocols=[...]
    )

    client_sock, client_info = bt_sock.accept()
    print("Accepted BT connection from ", client_info)

    kit = MotorKit()
    motors = [
        kit.motor1,
        kit.motor2,
        kit.motor3,
        kit.motor4,
    ]

    for m in motors:
        m.throttle = 1.0
        sleep(0.5)
        m.throttle = 0.0

    try:
        while True:
            data = client_sock.recv(1024)
            if not data:
                break
            print("Received: ", data)
    except:
        pass
